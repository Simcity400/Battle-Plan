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
//! Wings are visited in the recorded `CStrategicAir+0x98` registration order,
//! region by region: the viewing country's own wings, then the two country
//! arrays `140c43b30` builds from the recorded donor, wartime-ally and enemy
//! vectors, each in native order, and the viewer's volunteer hosts. A
//! transfer is rejected only when its base resolves in the recorded registry
//! witnesses.
//!
//! Known divergences from `14122efd0`: natively, `GroupAirWingsByScenario`
//! files eligible wings into per-region arrays of at most 50, one set for the
//! viewer and array A and a separate set for array B, and the builder draws
//! wings from each at random with a wall-clock seed. Here the wings of both
//! sides are taken as one stream in visit order (the first wings fill each
//! scenario limit) with a fixed seed. L6c ports the separate side arrays.
//!
//! Each wing then passes
//! `IsAirWingScenarioEligible@140F66100`'s own gates and target scans and the
//! native count law on the recorded mission, base, modifier and content
//! operands (`aircraft_count`, `air_reachability`). The tactical target scan
//! needs the land-combat country sets and is not yet decided, so CAS and
//! tactical-bomber wings stay omitted with a diagnostic. Training (0x800) is
//! not a registered-mission category: `14122EFD0` draws the viewer's own
//! training wings by their base's region, only in a region with no other
//! flight, as planes / TRANSPORT_DIVISION_FACTOR fighter patrols. That pass
//! is not ported, so training wings are not drawn (their pool bases are
//! witnessed for it). The replay does not
//! record hostile air-intel estimates, designer/graphical/cosmetic overrides,
//! scenario roster/outcome or clock; we use the largest recorded variant and
//! the default scenario outcome.
//! Seed 0x41495250, XOR wing ID/region/scenario ordinal, supplies the engine's
//! integer mixer, province, orientation and phase. This is a stable per-hour
//! presentation, not recovered game state. Opposing fighter/bomber pairing,
//! current CAS target and bombing-strength outcome cannot be recovered; those
//! wings use their independent default mission scenarios. Missiles and carrier
//! basing need other inputs and are skipped. Exact eligibility, count and
//! dogfight agreement require a recorded scenario/mission-effective census,
//! not a pixel refit. At both reference hours Germany has no participants and
//! no assigned own missions: exactly zero flight slots. The native pairK census
//! confirms an empty German roster at heights 50..310, with air map mode both
//! off and on, after allowing simulation to advance.
//!
//! The airborne manager has no dedicated height define or gate in its update.
//! We retain the shared DRAW_MAP_OBJECTS_CUTOFF define. Its airborne-specific
//! camera/fade behaviour remains UNVERIFIED pending a draw-bearing height sweep.
//! The supplied frame33840 has ZERO airborne PDX draws (all conventional mesh
//! diffuse resources match buildings/bridges/trains/map frame), so it cannot
//! settle altitude/scale/formation. No captured transform is an input here.
//! Frame-rate smoothing, propeller animation, gun/bomb/death particles and
//! success/failure transitions are not reconstructed. Authored rig transforms
//! are sampled at their frame rate and interpolated; the route banking uses
//! its tangent, with the unrecorded smoothing history omitted.

use std::{collections::BTreeMap, io};

use installed_content::{
    ResolvedLayers,
    defines::Defines,
    map::{MapData, ProvinceKind},
    mesh::{self, Transform},
    regions::StrategicRegion,
};
use replay::{
    frame::DecodedFrame,
    rows::{
        AIR_MISSION_LAY_MINES, AIR_MISSION_ZERO_PLANE_FLAGS, AirCountryRelationRow,
        AirMissionInputRow, AirWingRow, tables,
    },
};

use super::{
    air_participants::{AirParticipants, CountryRelations, CountryVectors, ordered_participants},
    air_reachability::{
        self, Category, Causes, MineTargetSources, MissionCountry, NavalCombatTargetSources,
        NavalOccupantSources, PortTargetSources, RegionFractions, ScoutTargetSources,
        StrategicTargetSources, TargetAdmission, TargetPredicates, Wars,
    },
    air_registry_sources::AirRegistrySources,
    aircraft_count::{SortieDefines, SortieSources},
    borders::HeightField,
    counters::CounterContent,
    models::{ModelField, ModelInstance, ModelLibrary},
    orders::ProvinceGeometry,
    perspective::Vision,
    tables::{Available, core, rows},
    visibility::Families,
};

pub const SEED: u32 = 0x4149_5250;
pub const CLOCK_SPEED: f32 = 0.22;

/// The master `DRAW_MAP_OBJECTS_CUTOFF` over every map object.
pub fn draws(camera: &super::camera::Camera) -> bool {
    camera.height < camera.defines.map_objects_cutoff
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
    /// Equipment names by the lexer's token identity.
    keys: crate::table_access::TokenKeys,
    rigs: BTreeMap<String, Rig>,
    /// Each strategic region's provinces, from content.
    region_provinces: BTreeMap<u32, Vec<u32>>,
    /// CStrategicRegion+0x98/+0xA0 land and sea fractions.
    region_fractions: BTreeMap<u32, RegionFractions>,
    province_kinds: BTreeMap<u32, ProvinceKind>,
    sortie_defines: Result<SortieDefines, String>,
}

/// The exact suffix ladder under each available prefix. Missing recorded
/// override levels are reported, never substituted with a graphical culture.
pub(crate) fn candidates(tag: &str, name: &str, archetype: &str, sprite: &str) -> Vec<String> {
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

impl AircraftAssets {
    pub fn load(
        layers: &ResolvedLayers,
        map: &MapData,
        defines: &Defines,
        regions: &BTreeMap<u32, StrategicRegion>,
        warnings: &mut Vec<String>,
    ) -> io::Result<Self> {
        // The content's equipment types, `duplicate_archetypes` copies
        // included: each copy's archetype and parent name other copies, and
        // a duplicate's own sprite replaces its copies' (`140C84DD0`).
        let mut equipment: BTreeMap<String, Equipment> = installed_content::equipment::read(layers)?
            .types
            .into_values()
            .map(|kind| {
                (
                    kind.name,
                    Equipment {
                        archetype: kind.archetype.unwrap_or_default(),
                        sprite: kind.sprite.unwrap_or_default(),
                        parent: kind.parent.unwrap_or_default(),
                    },
                )
            })
            .collect();
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
                    let fixed = mesh::balanced_blocks(&bytes);
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
        let province_kinds: BTreeMap<_, _> = map
            .definitions
            .iter()
            .map(|(&id, definition)| (id, definition.kind))
            .collect();
        let (region_provinces, region_fractions) = region_membership(
            regions
                .iter()
                .map(|(&id, region)| (id, region.provinces.clone())),
            &province_kinds,
            warnings,
        );
        Ok(Self {
            library,
            keys: crate::table_access::TokenKeys::new(equipment.keys()),
            equipment,
            rigs,
            region_provinces,
            region_fractions,
            province_kinds,
            sortie_defines: SortieDefines::load(defines).map_err(|error| error.to_string()),
        })
    }

    /// The mesh for the equipment type a token spells, if content has one.
    fn resolve(&self, tag: &str, spelling: &str) -> Result<Option<usize>, String> {
        let Some(name) = self.keys.resolve(spelling)? else {
            return Ok(None);
        };
        let eq = &self.equipment[name];
        Ok(candidates(tag, name, &eq.archetype, &eq.sprite)
            .iter()
            .find_map(|name| self.library.slot(name)))
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

type RegionMembership = (BTreeMap<u32, Vec<u32>>, BTreeMap<u32, RegionFractions>);

/// `CStrategicRegion::AddProvince@140E0F2B0` skips a listed ID the province
/// table does not hold and counts every other listing, duplicates included,
/// in the region's total and its land (kind bit 0) or sea (bits 0 and 1
/// clear) count. The skips are counted in one warning.
fn region_membership(
    regions: impl Iterator<Item = (u32, Vec<u32>)>,
    kinds: &BTreeMap<u32, ProvinceKind>,
    warnings: &mut Vec<String>,
) -> RegionMembership {
    let (mut members, mut fractions) = (BTreeMap::new(), BTreeMap::new());
    let mut skipped = 0_usize;
    for (region, listed) in regions {
        let before = listed.len();
        let provinces: Vec<u32> = listed
            .into_iter()
            .filter(|province| kinds.contains_key(province))
            .collect();
        skipped += before - provinces.len();
        let count = |wanted| {
            provinces
                .iter()
                .filter(|province| kinds.get(province) == Some(&wanted))
                .count() as u64
        };
        let region_fractions = air_reachability::region_fractions(
            provinces.len() as u64,
            count(ProvinceKind::Land),
            count(ProvinceKind::Sea),
        );
        fractions.insert(region, region_fractions);
        members.insert(region, provinces);
    }
    if skipped != 0 {
        warnings.push(format!(
            "strategic regions list {skipped} provinces missing from the map definitions; the game skips them"
        ));
    }
    (members, fractions)
}

/// This hour's F66100 target scans, built once and shared by every wing,
/// with the content fractions, the DLC mask and the mine cap they meet.
struct Scans<'a> {
    port: Option<PortTargetSources<'a>>,
    strategic: Option<StrategicTargetSources<'a>>,
    scout: Option<ScoutTargetSources<'a>>,
    naval_combat: Option<NavalCombatTargetSources<'a>>,
    occupants: Option<NavalOccupantSources<'a>>,
    mine: Option<MineTargetSources<'a>>,
    fractions: &'a BTreeMap<u32, RegionFractions>,
    dlc_mask: Option<u32>,
    mine_maximum_raw: i64,
}

impl Scans<'_> {
    /// F66100's target chain for a wing over its mission region.
    fn admission(
        &self,
        wing: &AirWingRow,
        country: MissionCountry,
        region: u32,
        inputs: &AirMissionInputRow,
    ) -> TargetAdmission {
        air_reachability::target_admission(
            wing.mission_mask,
            inputs.reached_count,
            self.fractions.get(&region).copied(),
            self.dlc_mask,
            self.predicates(wing, country, region, inputs),
        )
    }

    /// Only the scans `target_admission` reads for this wing's mask run.
    fn predicates(
        &self,
        wing: &AirWingRow,
        country: MissionCountry,
        region: u32,
        inputs: &AirMissionInputRow,
    ) -> TargetPredicates {
        let mask = air_reachability::scanned_bits(wing.mission_mask);
        let reached = &inputs.reached_provinces;
        let mut targets = TargetPredicates::default();
        if inputs.reached_count == 0 {
            return targets;
        }
        if mask & (1 << 8) != 0 {
            targets.port = self
                .port
                .as_ref()
                .and_then(|scan| scan.admit(country, reached));
        }
        if mask & 0x1_0008 != 0
            && let Some(scan) = &self.strategic
        {
            let strategic = scan.admit(country, region, reached);
            targets.strategic = strategic.found;
            targets.strategic_railway_gun = strategic.railway_gun;
        }
        if mask & (1 << 14) != 0 {
            targets.scout = self
                .scout
                .as_ref()
                .and_then(|scan| scan.admit(wing, region, inputs.definition_map_flags));
        }
        if mask & 0x90 != 0 {
            targets.naval_combat = self
                .naval_combat
                .as_ref()
                .and_then(|scan| scan.admit_combat(country, reached));
            targets.naval_occupants = self
                .occupants
                .as_ref()
                .map(|scan| scan.admit(country, reached))
                .unwrap_or_default();
        }
        if mask & AIR_MISSION_LAY_MINES != 0 {
            targets.mine = self
                .mine
                .as_ref()
                .and_then(|scan| scan.admit(wing, region, self.mine_maximum_raw));
        }
        targets
    }
}

/// Wings omitted for an undecided target, counted by reason.
#[derive(Default)]
struct Undecided {
    by_reason: BTreeMap<&'static str, usize>,
    railway_gun_only: usize,
}

impl Undecided {
    fn record(&mut self, causes: Causes) {
        self.railway_gun_only += usize::from(causes.railway_gun_only());
        for cause in causes.iter() {
            *self.by_reason.entry(cause.reason()).or_default() += 1;
        }
    }

    fn diagnostics(&self) -> impl Iterator<Item = String> + '_ {
        self.by_reason
            .iter()
            .map(|(reason, count)| {
                format!("{count} wings omitted: mission target admission unknown: {reason}")
            })
            .chain((self.railway_gun_only != 0).then(|| {
                format!(
                    "{} wings omitted only for the unrecorded railway-gun target arms",
                    self.railway_gun_only
                )
            }))
    }
}

/// `14122EB00`'s scenario family; base-equipment missions do not fly.
fn family(
    mask: u32,
    last_target: i32,
    definition_flags: u64,
) -> Option<(&'static str, &'static str)> {
    match air_reachability::category(mask, last_target, definition_flags)? {
        Category::Transport => Some(("transport", "supplying")),
        Category::Scout => Some(("scout", "patrol")),
        Category::BomberFlight => {
            Some(("bomber", if mask & 0x202 != 0 { "CAS" } else { "bombing" }))
        }
        Category::Fighter => Some(("fighter", "patrol")),
        Category::BaseEquipment => None,
    }
}

/// F66100's direct rejections before its target scans: no mission, the veto,
/// the counter over its limit, or an inactive regional mission (training
/// flies without one).
fn passes_initial_gate(wing: &AirWingRow) -> bool {
    wing.mission_mask != 0
        && wing.eligibility_veto == 0
        && wing.eligibility_counter <= wing.eligibility_limit
        && ((wing.mission_region.is_some() && wing.mission_active != 0)
            || wing.mission_mask & 0x800 != 0)
}

/// A whole wing batch; the regional cap is checked before calling this.
fn formation_count(planes: i32) -> (usize, usize) {
    let n = (planes / 30).max(i32::from(planes > 0)) as usize;
    if n >= 3 { (3, n / 3) } else { (1, n) }
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

/// `140c43b30`'s two arrays for the viewing country. Without complete
/// relation vectors and volunteer hosts only its own wings fly; a partial
/// vector cannot give the native first-occurrence order. Only the vectors
/// the arrays use are looked up, by binary search of the key-sorted rows.
fn participant_countries(
    frame: &DecodedFrame,
    ours: u32,
    links: &BTreeMap<u32, u32>,
) -> io::Result<(AirParticipants, Option<String>)> {
    let (Available::Complete(relation_rows), Available::Complete(host_rows)) = (
        rows::<tables::AirCountryRelations>(frame)?,
        rows::<tables::VolunteerHosts>(frame)?,
    ) else {
        return Ok((
            AirParticipants::default(),
            Some("foreign aircraft omitted: complete country air relations and volunteer hosts are required".into()),
        ));
    };
    let mut hosts: Vec<_> = host_rows
        .iter()
        .filter(|row| links.get(&row.country) == Some(&ours))
        .map(|row| (row.position, row.host))
        .collect();
    hosts.sort_unstable();
    let hosts: Vec<_> = hosts.into_iter().map(|(_, host)| host).collect();
    let relations = RelationRows {
        rows: relation_rows,
        links,
    };
    let participants = ordered_participants(ours, &relations, &hosts, links)?;
    Ok((participants, None))
}

/// AirCountryRelations rows, sorted by `(country, kind, ordinal)`.
struct RelationRows<'a> {
    rows: &'a [AirCountryRelationRow],
    links: &'a BTreeMap<u32, u32>,
}

impl RelationRows<'_> {
    /// A country's rows of kinds `first..=last`: a binary search for the
    /// first, then a scan of the rows the caller reads anyway.
    fn span(&self, slot: u32, first: u8, last: u8) -> &[AirCountryRelationRow] {
        let start = self
            .rows
            .partition_point(|row| (row.country, row.kind) < (slot, first));
        let len = self.rows[start..]
            .iter()
            .take_while(|row| (row.country, row.kind) <= (slot, last))
            .count();
        &self.rows[start..start + len]
    }

    fn push(&self, vector: &mut Vec<u32>, row: &AirCountryRelationRow) -> io::Result<()> {
        if row.ordinal as usize != vector.len() || !self.links.contains_key(&row.other) {
            return Err(invalid(
                "air relation positions are not contiguous or name an unlinked country",
            ));
        }
        vector.push(row.other);
        Ok(())
    }
}

impl CountryVectors for RelationRows<'_> {
    fn vectors(&self, slot: u32) -> io::Result<CountryRelations> {
        let mut relations = CountryRelations::default();
        for row in self.span(slot, u8::MIN, u8::MAX) {
            let vector = match row.kind {
                AirCountryRelationRow::DONOR => &mut relations.reciprocal_donors,
                AirCountryRelationRow::ALLY => &mut relations.wartime_allies,
                AirCountryRelationRow::ENEMY => &mut relations.enemies,
                _ => return Err(invalid("air relation has an invalid kind")),
            };
            self.push(vector, row)?;
        }
        Ok(relations)
    }

    fn donors(&self, slot: u32, donors: &mut Vec<u32>) -> io::Result<()> {
        donors.clear();
        let donor = AirCountryRelationRow::DONOR;
        for row in self.span(slot, donor, donor) {
            self.push(donors, row)?;
        }
        Ok(())
    }
}

/// Registered wings in `14122efd0`'s visit order: region IDs ascending, then
/// the viewing country's wings and those of arrays A and B, each country's in
/// registration order. A country in both arrays is visited twice, as natively.
fn visit_order<'a>(
    wings: &'a [AirWingRow],
    registrations: &[replay::rows::AirRegionalMissionRow],
    links: &BTreeMap<u32, u32>,
    ours: u32,
    participants: &AirParticipants,
) -> io::Result<Vec<(u32, u32, &'a AirWingRow)>> {
    let by_id: BTreeMap<_, _> = wings.iter().map(|wing| (wing.id, wing)).collect();
    let mut by_region = BTreeMap::<u32, BTreeMap<u32, Vec<(u32, &AirWingRow)>>>::new();
    for registration in registrations {
        let wing = *by_id
            .get(&registration.wing)
            .ok_or_else(|| invalid("registered air mission lacks its wing"))?;
        let country = *links
            .get(&wing.owner)
            .ok_or_else(|| invalid("registered air mission owner is not linked"))?;
        let region = wing
            .mission_region
            .ok_or_else(|| invalid("registered air mission has no region"))?;
        by_region
            .entry(region)
            .or_default()
            .entry(country)
            .or_default()
            .push((registration.ordinal, wing));
    }
    let mut ordered = Vec::with_capacity(registrations.len());
    for (region, mut by_country) in by_region {
        for wings in by_country.values_mut() {
            wings.sort_unstable_by_key(|&(ordinal, _)| ordinal);
            if wings
                .iter()
                .enumerate()
                .any(|(expected, &(ordinal, _))| ordinal as usize != expected)
            {
                return Err(invalid("regional air mission ordinals are not contiguous"));
            }
        }
        let countries = std::iter::once(ours).chain(
            participants
                .a
                .iter()
                .chain(&participants.b)
                .filter_map(|raw| links.get(raw).copied()),
        );
        for country in countries {
            if let Some(wings) = by_country.get(&country) {
                ordered.extend(wings.iter().map(|&(_, wing)| (country, region, wing)));
            }
        }
    }
    Ok(ordered)
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
        let variants = rows::<tables::VariantDefinitions>(frame)?;
        let variants: BTreeMap<_, _> = variants.rows().iter().map(|r| (r.id, r)).collect();
        let tokens = crate::table_access::TokenNames::new(frame)?;
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
        // Native manager +108 is the viewing country; the fog-of-war switch
        let ours = vision.ours();
        let (participants, gap) = participant_countries(frame, ours, &links)?;
        out.diagnostics.extend(gap);
        let (Available::Complete(wings), Available::Complete(registrations), Some(registry)) = (
            rows::<tables::AirWings>(frame)?,
            rows::<tables::AirRegionalMissions>(frame)?,
            AirRegistrySources::read(frame)?,
        ) else {
            out.diagnostics.push(
                "flying aircraft omitted: complete wings, mission registrations and base registry witnesses are required".into(),
            );
            return Ok(out);
        };
        let sources = match SortieSources::read(frame)? {
            Ok(sources) => sources,
            Err(reason) => {
                out.diagnostics
                    .push(format!("flying aircraft omitted: {reason}"));
                return Ok(out);
            }
        };
        let defines = match &assets.sortie_defines {
            Ok(defines) => defines,
            Err(error) => {
                out.diagnostics.push(format!(
                    "flying aircraft omitted: content sortie defines unavailable: {error}"
                ));
                return Ok(out);
            }
        };
        let dlc_mask = match rows::<tables::SimulationFeatures>(frame)? {
            Available::Complete([features]) => Some(features.dlc_mask),
            _ => None,
        };
        let mission_countries =
            air_reachability::MissionCountrySources::read(frame, &links, &registry)?;
        // Shared by every scan: one war census and one family set per hour.
        let wars = Wars::read(frame, &links)?;
        let families = Families::read(frame, &links).ok();
        let fog_families = families
            .clone()
            .and_then(|families| families.with_attaches(frame, &links).ok());
        let needs = |bits: u32| {
            wings
                .iter()
                .any(|wing| air_reachability::scanned_bits(wing.mission_mask) & bits != 0)
        };
        let scans = Scans {
            port: needs(1 << 8)
                .then(|| PortTargetSources::read(frame, &wars))
                .transpose()?,
            strategic: needs(0x1_0008)
                .then(|| StrategicTargetSources::read(frame, &wars, &assets.region_provinces))
                .transpose()?,
            scout: needs(1 << 14)
                .then(|| {
                    ScoutTargetSources::read(
                        frame,
                        &links,
                        &wars,
                        fog_families.as_ref(),
                        &assets.region_provinces,
                    )
                })
                .transpose()?,
            naval_combat: needs(0x90)
                .then(|| NavalCombatTargetSources::read(frame, &links, &wars, families.as_ref()))
                .transpose()?,
            occupants: needs(0x90)
                .then(|| NavalOccupantSources::read(frame, &wars, &assets.province_kinds))
                .transpose()?,
            mine: needs(AIR_MISSION_LAY_MINES)
                .then(|| MineTargetSources::read(frame, &links))
                .transpose()?,
            fractions: &assets.region_fractions,
            dlc_mask,
            mine_maximum_raw: defines.mine_maximum_raw,
        };
        let mut counts: BTreeMap<(u32, String), usize> = BTreeMap::new();
        let mut undecided = Undecided::default();
        for (slot, region, wing) in visit_order(wings, registrations, &links, ours, &participants)?
        {
            if !passes_initial_gate(wing) {
                continue;
            }
            let Some(inputs) = sources.mission(wing.id) else {
                out.diagnostics
                    .push(format!("wing {} lacks its mission operands", wing.id));
                continue;
            };
            // F66100 lets a wing without planes fly only for these definitions.
            if wing.planes == 0 && inputs.definition_map_flags & AIR_MISSION_ZERO_PLANE_FLAGS == 0 {
                continue;
            }
            let Some((family, mission)) = family(
                wing.mission_mask,
                inputs.last_target_province,
                inputs.definition_map_flags,
            ) else {
                continue;
            };
            let owner = if wing.country_override > 0 {
                wing.country_override as u32
            } else {
                wing.owner
            };
            let Some(&model_slot) = links.get(&owner) else {
                continue;
            };
            // 14122efd0 only scans our training fallback and passes no scout
            // destination list for allied/enemy countries. 140f66100 rejects
            // a transfer whose base resolves, rather than flying from it.
            if (slot != ours && wing.mission_mask & (0x800 | 0x4000) != 0)
                || registry.transfer_resolves(wing.base_override)
            {
                continue;
            }
            let Some(base) = registry.bases.get(&wing.air_base) else {
                out.diagnostics
                    .push(format!("wing {} pool base does not resolve", wing.id));
                continue;
            };
            let Some(mission_country) = mission_countries.resolve(wing) else {
                out.diagnostics.push(format!(
                    "wing {} mission country cannot be derived from this hour's sources",
                    wing.id
                ));
                continue;
            };
            let planes = match sources.count(
                wing,
                inputs,
                base,
                mission_country.slot,
                (owner, model_slot),
                defines,
            ) {
                Ok(planes) => planes,
                Err(reason) => {
                    out.diagnostics
                        .push(format!("wing {} count omitted: {reason}", wing.id));
                    continue;
                }
            };
            if planes <= 0 {
                continue;
            }
            match sources.host_admits(wing, base) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(reason) => {
                    out.diagnostics
                        .push(format!("wing {} omitted: {reason}", wing.id));
                    continue;
                }
            }
            match scans.admission(wing, mission_country, region, inputs) {
                TargetAdmission::Allowed => {}
                TargetAdmission::Blocked => continue,
                TargetAdmission::Unknown(causes) => {
                    undecided.record(causes);
                    continue;
                }
            }
            let Some(start) = base.state.and_then(|state| content.air_base(state)) else {
                out.diagnostics.push(format!(
                    "wing {} has no authored land-base slot (carrier/missing base)",
                    wing.id
                ));
                continue;
            };
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
            let name = match tokens.name(variant.equipment_token) {
                Ok(name) => name,
                Err(reason) => {
                    out.diagnostics
                        .push(format!("wing {} equipment type: {reason}", wing.id));
                    continue;
                }
            };
            let mesh = match assets.resolve(tags.get(&model_slot).map_or("", String::as_str), name)
            {
                Ok(mesh) => mesh,
                Err(reason) => {
                    out.diagnostics
                        .push(format!("wing {} equipment type: {reason}", wing.id));
                    continue;
                }
            };
            let Some(mesh) = mesh else {
                out.diagnostics.push(format!(
                    "wing {} unresolved aircraft equipment {name}",
                    wing.id
                ));
                continue;
            };
            let (formation, batches) = formation_count(planes);
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
        out.diagnostics.extend(undecided.diagnostics());
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
                        ..ModelInstance::default()
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
    fn wing(id: u64, owner: u32, region: u32) -> AirWingRow {
        AirWingRow {
            id,
            owner,
            mission_region: Some(region),
            ..Default::default()
        }
    }

    fn registration(wing: u64, ordinal: u32) -> replay::rows::AirRegionalMissionRow {
        replay::rows::AirRegionalMissionRow { wing, ordinal }
    }

    #[test]
    fn wings_fly_region_by_region_ours_first_then_both_arrays_in_registration_order() {
        let wings = [
            wing(1, 7, 5),
            wing(2, 7, 5),
            wing(3, 8, 5),
            wing(4, 9, 4),
            wing(5, 10, 5),
        ];
        let registrations = [
            registration(1, 1),
            registration(2, 0),
            registration(3, 0),
            registration(4, 0),
            registration(5, 0),
        ];
        let links = BTreeMap::from([(7, 7), (8, 8), (9, 9), (10, 10), (18, 8)]);
        let participants = AirParticipants {
            a: vec![18, 9],
            b: vec![8],
        };
        let order = visit_order(&wings, &registrations, &links, 7, &participants).unwrap();
        let ids: Vec<_> = order
            .iter()
            .map(|&(_, region, wing)| (region, wing.id))
            .collect();
        // Country 10 is no participant; country 8 sits in both arrays.
        assert_eq!(ids, [(4, 4), (5, 2), (5, 1), (5, 3), (5, 3)]);
        let gap = [registration(1, 1), registration(2, 2)];
        assert!(visit_order(&wings, &gap, &links, 7, &participants).is_err());
    }

    #[test]
    fn recorded_vectors_and_hosts_build_the_viewpoints_arrays() {
        use replay::{
            frame::{Metrics, TableView},
            rows::{TableId, TableRows, VolunteerHostRow},
        };
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 42,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
            metrics: Metrics::default(),
        };
        let links = BTreeMap::from([(101, 7), (202, 8), (203, 8), (303, 9), (404, 10)]);
        let relation = |country, kind, ordinal, other| AirCountryRelationRow {
            country,
            kind,
            ordinal,
            other,
        };
        *frame.tables[TableId::AirCountryRelations.index()] =
            TableView::Complete(TableRows::AirCountryRelations(vec![
                relation(7, 0, 0, 202),
                relation(7, 0, 1, 203),
                relation(7, 2, 0, 404),
                relation(8, 1, 0, 303),
            ]));
        *frame.tables[TableId::VolunteerHosts.index()] =
            TableView::Complete(TableRows::VolunteerHosts(vec![VolunteerHostRow {
                country: 101,
                position: 0,
                host: 202,
                ..Default::default()
            }]));
        let (arrays, gap) = participant_countries(&frame, 7, &links).unwrap();
        assert!(gap.is_none());
        assert_eq!(arrays.a, [202, 303]);
        assert_eq!(arrays.b, [404]);
        // A gap in a vector's positions is refused.
        *frame.tables[TableId::AirCountryRelations.index()] =
            TableView::Complete(TableRows::AirCountryRelations(vec![relation(7, 2, 1, 404)]));
        assert!(participant_countries(&frame, 7, &links).is_err());
        *frame.tables[TableId::AirCountryRelations.index()] = TableView::Unavailable {
            reason: "test".into(),
        };
        let (arrays, gap) = participant_countries(&frame, 7, &links).unwrap();
        assert!(arrays.a.is_empty() && arrays.b.is_empty() && gap.is_some());
    }
    /// Countries `1..=n` at war in two blocs: each allied to the rest of its
    /// bloc, the enemy of the other and given two donors from its own bloc.
    /// The viewing country 1 volunteers to host country `n`.
    fn bloc_war(n: u32) -> (DecodedFrame, BTreeMap<u32, u32>) {
        use replay::{
            frame::{Metrics, TableView},
            rows::{TableId, TableRows, VolunteerHostRow},
        };
        let half = n / 2;
        let bloc = |country: u32| {
            if country <= half {
                1..=half
            } else {
                half + 1..=n
            }
        };
        let mut rows = Vec::new();
        for country in 1..=n {
            let own = bloc(country);
            let (first, len) = (*own.start(), own.end() - own.start() + 1);
            let donors = [1, 2].map(|step| first + (country - first + step) % len);
            let others = if country <= half {
                half + 1..=n
            } else {
                1..=half
            };
            let vectors: [Vec<u32>; 3] = [
                donors.into(),
                own.filter(|&other| other != country).collect(),
                others.collect(),
            ];
            for (kind, vector) in [
                AirCountryRelationRow::DONOR,
                AirCountryRelationRow::ALLY,
                AirCountryRelationRow::ENEMY,
            ]
            .into_iter()
            .zip(vectors)
            {
                rows.extend(
                    (0..)
                        .zip(vector)
                        .map(|(ordinal, other)| AirCountryRelationRow {
                            country,
                            kind,
                            ordinal,
                            other,
                        }),
                );
            }
        }
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 42,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::AirCountryRelations.index()] =
            TableView::Complete(TableRows::AirCountryRelations(rows));
        *frame.tables[TableId::VolunteerHosts.index()] =
            TableView::Complete(TableRows::VolunteerHosts(vec![VolunteerHostRow {
                country: 1,
                position: 0,
                host: n,
                ..Default::default()
            }]));
        (frame, (1..=n).map(|country| (country, country)).collect())
    }

    #[test]
    fn a_bloc_war_reads_only_the_vectors_the_arrays_use() {
        let (frame, links) = bloc_war(40);
        let (arrays, gap) = participant_countries(&frame, 1, &links).unwrap();
        assert!(gap.is_none());
        // The same arrays as from every country's complete vectors.
        let Available::Complete(rows) = rows::<tables::AirCountryRelations>(&frame).unwrap() else {
            unreachable!("complete fixture")
        };
        let mut complete = BTreeMap::<u32, CountryRelations>::new();
        for row in rows {
            let relations = complete.entry(row.country).or_default();
            match row.kind {
                AirCountryRelationRow::DONOR => &mut relations.reciprocal_donors,
                AirCountryRelationRow::ALLY => &mut relations.wartime_allies,
                _ => &mut relations.enemies,
            }
            .push(row.other);
        }
        struct Complete(BTreeMap<u32, CountryRelations>);
        impl CountryVectors for Complete {
            fn vectors(&self, slot: u32) -> io::Result<CountryRelations> {
                Ok(self.0.get(&slot).cloned().unwrap_or_default())
            }
            fn donors(&self, slot: u32, donors: &mut Vec<u32>) -> io::Result<()> {
                *donors = self.vectors(slot)?.reciprocal_donors;
                Ok(())
            }
        }
        let expected = ordered_participants(1, &Complete(complete), &[40], &links).unwrap();
        assert_eq!(arrays, expected);
        // Own donors, then each ally and its donors; the host, then its allies.
        assert_eq!(arrays.a[..4], [2, 3, 4, 5]);
        assert_eq!((arrays.a.len(), arrays.b.len()), (40, 40));
        assert_eq!(arrays.b[..3], [21, 22, 23]);
    }

    /// The scale matrix, in release:
    /// `cargo test -p viewer --release --bin viewer -- --ignored bloc_war_participants`.
    #[test]
    #[ignore]
    fn bloc_war_participants_at_1500_and_5000_countries() {
        for n in [1_500, 5_000] {
            let (frame, links) = bloc_war(n);
            let mut best = std::time::Duration::MAX;
            let mut arrays = AirParticipants::default();
            for _ in 0..5 {
                let started = std::time::Instant::now();
                arrays = participant_countries(&frame, 1, &links).unwrap().0;
                best = best.min(started.elapsed());
            }
            eprintln!(
                "participants, {n} countries at war in two blocs: {best:?} (best of 5), arrays of {} and {}",
                arrays.a.len(),
                arrays.b.len()
            );
        }
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
    fn supported_missions_follow_priority_and_training_does_not_fly_here() {
        assert_eq!(family(0x800, 17, 0), None);
        assert_eq!(family(4, 0, 0), Some(("fighter", "patrol")));
        assert_eq!(family(2 | 8, 17, 0), Some(("bomber", "CAS")));
        assert_eq!(family(1 << 14, 17, 0), Some(("scout", "patrol")));
        assert_eq!(family(1 << 10, 17, 0), Some(("transport", "supplying")));
        assert_eq!(family(1 << 10, 0, 0), None);
        assert_eq!(family(0, 0, 0), None);
    }

    #[test]
    fn regions_skip_undefined_provinces_and_count_duplicates() {
        let kinds = BTreeMap::from([
            (1, ProvinceKind::Land),
            (2, ProvinceKind::Sea),
            (3, ProvinceKind::Lake),
        ]);
        let mut warnings = Vec::new();
        let (members, fractions) = region_membership(
            [(7, vec![1, 2, 2, 3, 99]), (8, vec![98])].into_iter(),
            &kinds,
            &mut warnings,
        );
        assert_eq!(members[&7], [1, 2, 2, 3]);
        assert!(members[&8].is_empty());
        assert_eq!(
            fractions[&7],
            RegionFractions {
                land_raw: 25_000,
                sea_raw: 50_000
            }
        );
        assert_eq!(fractions[&8], RegionFractions::default());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("list 2 provinces"));
    }

    mod scans {
        use super::*;
        use replay::{
            frame::{Metrics, TableView},
            rows::{
                CountryRow, DivisionRow, NavalBaseRow, NavalCombatDetailsRow, NavalCombatRow,
                NavalCombatShipRow, NavalMineLevelRow, ProvinceRow, ProvinceUnitOrderRow, StateRow,
                TableId, TableRows, TableSchema, TaskForceRow, WarRelationRow,
            },
        };

        const LAND: RegionFractions = RegionFractions {
            land_raw: 100_000,
            sea_raw: 0,
        };
        const SEA: RegionFractions = RegionFractions {
            land_raw: 0,
            sea_raw: 100_000,
        };

        fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
            *frame.tables[T::ID.index()] = TableView::Complete(TableRows::from_rows::<T>(rows));
        }

        /// Complete empty tables, with `countries` slots owning their numbers.
        fn frame(countries: u32) -> DecodedFrame {
            let mut frame = DecodedFrame {
                branch: 1,
                hour: 101,
                tables: std::array::from_fn(|index| {
                    TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
                }),
                metrics: Metrics::default(),
            };
            set::<tables::Countries>(
                &mut frame,
                (1..=countries)
                    .map(|slot| CountryRow {
                        id: slot,
                        canonical_slot: slot,
                        ..Default::default()
                    })
                    .collect(),
            );
            frame
        }

        fn war(first_country: u32, second_country: u32) -> WarRelationRow {
            WarRelationRow {
                first_country,
                second_country,
                ..Default::default()
            }
        }

        /// A hostile port at 412 and a region-7 mine level for country 1.
        fn port_and_mines() -> DecodedFrame {
            let mut frame = frame(3);
            set::<tables::Provinces>(
                &mut frame,
                vec![ProvinceRow {
                    id: 412,
                    controller: 3,
                    ..Default::default()
                }],
            );
            set::<tables::NavalBases>(
                &mut frame,
                vec![NavalBaseRow {
                    province: 412,
                    ..Default::default()
                }],
            );
            set::<tables::ProvinceUnitOrder>(
                &mut frame,
                vec![ProvinceUnitOrderRow {
                    province: 412,
                    naval: vec![42],
                    ..Default::default()
                }],
            );
            set::<tables::TaskForces>(
                &mut frame,
                vec![TaskForceRow {
                    id: 42,
                    province: 412,
                    logical_owner: 3,
                    ..Default::default()
                }],
            );
            set::<tables::WarRelations>(&mut frame, vec![war(1, 3)]);
            set::<tables::NavalMineLevels>(
                &mut frame,
                vec![NavalMineLevelRow {
                    country: 1,
                    region: 7,
                    raw_mines: 0,
                }],
            );
            frame
        }

        fn inputs(reached: Vec<u32>) -> AirMissionInputRow {
            AirMissionInputRow {
                reached_count: reached.len() as i32,
                reached_provinces: reached,
                ..Default::default()
            }
        }

        const COUNTRY: MissionCountry = MissionCountry { slot: 1 };

        #[test]
        fn predicates_reach_only_the_wings_whose_mask_reads_them() {
            let frame = port_and_mines();
            let links = BTreeMap::from([(1, 1), (3, 3)]);
            let wars = Wars::read(&frame, &links).unwrap();
            let fractions = BTreeMap::from([(7, LAND)]);
            let scans = Scans {
                port: Some(PortTargetSources::read(&frame, &wars).unwrap()),
                strategic: None,
                scout: None,
                naval_combat: None,
                occupants: None,
                mine: Some(MineTargetSources::read(&frame, &links).unwrap()),
                fractions: &fractions,
                dlc_mask: Some(0x10),
                mine_maximum_raw: 100_000,
            };
            let wing = |mission_mask| AirWingRow {
                owner: 1,
                mission_mask,
                ..Default::default()
            };
            let reached = inputs(vec![412]);
            let port = scans.predicates(&wing(1 << 8), COUNTRY, 7, &reached);
            assert_eq!((port.port, port.mine), (Some(true), None));
            let mines = scans.predicates(&wing(AIR_MISSION_LAY_MINES), COUNTRY, 7, &reached);
            assert_eq!((mines.port, mines.mine), (None, Some(true)));
            // 14122EB00's count gate and the bit-6 bypass read no scan.
            let empty = scans.predicates(&wing(1 << 8), COUNTRY, 7, &inputs(Vec::new()));
            assert_eq!(empty.port, None);
            let bypass = scans.predicates(&wing((1 << 8) | (1 << 6)), COUNTRY, 7, &reached);
            assert_eq!(bypass.port, None);
            // Past the bit 9/10 bypass F66100 reads no bit 12..16 scan, but
            // still the earlier ones.
            let transport = (1 << 10) | (1 << 8) | AIR_MISSION_LAY_MINES;
            let transport = scans.predicates(&wing(transport), COUNTRY, 7, &reached);
            assert_eq!((transport.port, transport.mine), (Some(true), None));
        }

        #[test]
        fn admission_takes_the_mission_regions_fractions() {
            let frame = port_and_mines();
            let links = BTreeMap::from([(1, 1), (3, 3)]);
            let wars = Wars::read(&frame, &links).unwrap();
            let fractions = BTreeMap::from([(7, LAND), (8, SEA)]);
            let scans = Scans {
                port: Some(PortTargetSources::read(&frame, &wars).unwrap()),
                strategic: None,
                scout: None,
                naval_combat: None,
                occupants: None,
                mine: None,
                fractions: &fractions,
                dlc_mask: Some(0),
                mine_maximum_raw: 100_000,
            };
            let wing = AirWingRow {
                owner: 1,
                mission_mask: 1 << 8,
                ..Default::default()
            };
            let reached = inputs(vec![412]);
            assert_eq!(
                scans.admission(&wing, COUNTRY, 7, &reached),
                TargetAdmission::Allowed
            );
            assert_eq!(
                scans.admission(&wing, COUNTRY, 8, &reached),
                TargetAdmission::Blocked
            );
            let TargetAdmission::Unknown(causes) = scans.admission(&wing, COUNTRY, 9, &reached)
            else {
                panic!("a region without fractions is undecided");
            };
            assert!(causes.iter().eq([air_reachability::Cause::Port]));
        }

        #[test]
        fn undecided_wings_are_one_diagnostic_per_reason_and_railway_guns_are_counted() {
            let railway = air_reachability::target_admission(
                (1 << 3) | (1 << 4),
                1,
                Some(RegionFractions {
                    land_raw: 50_000,
                    sea_raw: 50_000,
                }),
                Some(0x10),
                TargetPredicates {
                    strategic_railway_gun: true,
                    naval_combat: Some(false),
                    naval_occupants: air_reachability::NavalOccupants {
                        task_force: Some(false),
                        embarked: Some(false),
                        railway_gun: None,
                    },
                    ..Default::default()
                },
            );
            let tactical = air_reachability::target_admission(
                1 << 1,
                1,
                Some(LAND),
                Some(0),
                Default::default(),
            );
            let mut undecided = Undecided::default();
            for admission in [railway, railway, tactical] {
                let TargetAdmission::Unknown(causes) = admission else {
                    panic!("{admission:?}");
                };
                undecided.record(causes);
            }
            let lines: Vec<_> = undecided.diagnostics().collect();
            assert_eq!(lines.len(), 4);
            assert!(lines.iter().any(|line| line.starts_with("2 wings omitted")
                && line.contains(air_reachability::Cause::NavalRailwayGun.reason())));
            assert!(lines.iter().any(|line| line.starts_with("1 wings omitted")
                && line.contains(air_reachability::Cause::Tactical.reason())));
            assert_eq!(
                lines.last().unwrap(),
                "2 wings omitted only for the unrecorded railway-gun target arms"
            );
        }

        /// A synthetic hour at campaign density: 13,000 provinces (a third
        /// sea), 170 regions, 1,500 active countries with 60 wars per 100, 200
        /// naval combats of 40 ships, 2,000 naval bases, two task forces and
        /// 20 divisions per active country, and 5,000 registered missions of
        /// 16 reached provinces. 3,500 more countries without units, wars or
        /// provinces must not change a single war test or admission. Prints
        /// the scan build and per-hour predicate time; run with
        /// `--release --nocapture` to measure.
        #[test]
        fn target_scans_scale_with_missions_not_countries() {
            let run = |countries: u32| {
                let (frame, links, regions, wings) = dense_hour(countries);
                let kinds: BTreeMap<u32, ProvinceKind> = (1..=13_000)
                    .map(|id| {
                        (
                            id,
                            if id % 3 == 0 {
                                ProvinceKind::Sea
                            } else {
                                ProvinceKind::Land
                            },
                        )
                    })
                    .collect();
                let fractions: BTreeMap<u32, RegionFractions> = regions
                    .keys()
                    .map(|&region| {
                        (
                            region,
                            RegionFractions {
                                land_raw: 66_666,
                                sea_raw: 33_333,
                            },
                        )
                    })
                    .collect();
                let begin = std::time::Instant::now();
                let wars = Wars::read(&frame, &links).unwrap();
                let families = Families::read(&frame, &links).ok();
                let fog = families
                    .clone()
                    .and_then(|families| families.with_attaches(&frame, &links).ok());
                let scans = Scans {
                    port: Some(PortTargetSources::read(&frame, &wars).unwrap()),
                    strategic: Some(StrategicTargetSources::read(&frame, &wars, &regions).unwrap()),
                    scout: Some(
                        ScoutTargetSources::read(&frame, &links, &wars, fog.as_ref(), &regions)
                            .unwrap(),
                    ),
                    naval_combat: Some(
                        NavalCombatTargetSources::read(&frame, &links, &wars, families.as_ref())
                            .unwrap(),
                    ),
                    occupants: Some(NavalOccupantSources::read(&frame, &wars, &kinds).unwrap()),
                    mine: Some(MineTargetSources::read(&frame, &links).unwrap()),
                    fractions: &fractions,
                    dlc_mask: Some(0x10),
                    mine_maximum_raw: 100_000,
                };
                let built = begin.elapsed();
                let begin = std::time::Instant::now();
                let admissions: Vec<_> = wings
                    .iter()
                    .map(|(wing, input)| {
                        let country = MissionCountry { slot: wing.owner };
                        scans.admission(wing, country, wing.mission_region.unwrap(), input)
                    })
                    .collect();
                let scanned = begin.elapsed();
                let allowed = admissions
                    .iter()
                    .filter(|&&admission| admission == TargetAdmission::Allowed)
                    .count();
                println!(
                    "{countries} countries, {} missions: build {built:?}, admission {scanned:?}, {allowed} allowed",
                    wings.len()
                );
                (wars.tests.get(), admissions)
            };
            let (tests, admissions) = run(1_500);
            assert!(tests > 0);
            assert!(admissions.contains(&TargetAdmission::Allowed));
            assert_eq!(run(5_000), (tests, admissions));
        }

        type Hour = (
            DecodedFrame,
            BTreeMap<u32, u32>,
            BTreeMap<u32, Vec<u32>>,
            Vec<(AirWingRow, AirMissionInputRow)>,
        );

        fn dense_hour(countries: u32) -> Hour {
            const PROVINCES: u32 = 13_000;
            const ACTIVE: u32 = 1_500;
            let mut frame = frame(countries);
            let links = (1..=countries).map(|slot| (slot, slot)).collect();
            let owner = |id: u32| id % ACTIVE + 1;
            set::<tables::Provinces>(
                &mut frame,
                (1..=PROVINCES)
                    .map(|id| ProvinceRow {
                        id,
                        state: id / 10 + 1,
                        controller: owner(id / 10),
                        ..Default::default()
                    })
                    .collect(),
            );
            set::<tables::States>(
                &mut frame,
                (1..=PROVINCES / 10 + 1)
                    .map(|id| StateRow {
                        id,
                        controller: owner(id - 1),
                        ..Default::default()
                    })
                    .collect(),
            );
            let mut wars: Vec<_> = (1..=ACTIVE)
                .step_by(5)
                .flat_map(|first| {
                    (1..=3).map(move |step| war(first, (first + step * 7) % ACTIVE + 1))
                })
                .collect();
            wars.sort_unstable_by_key(|row| (row.first_country, row.second_country));
            wars.dedup_by_key(|row| (row.first_country, row.second_country));
            set::<tables::WarRelations>(&mut frame, wars);
            let sea = |index: u32| (index * 3) % PROVINCES + 3;
            let mut combats: Vec<_> = (0..200_u32)
                .map(|index| NavalCombatRow {
                    id: u64::from(index + 1),
                    province: sea(index * 17),
                    ..Default::default()
                })
                .collect();
            combats.sort_unstable_by_key(|row| row.id);
            let details = combats
                .iter()
                .map(|combat| {
                    let mut detail = NavalCombatDetailsRow {
                        combat: combat.id,
                        province: combat.province,
                        ..Default::default()
                    };
                    detail.sides[0].summary.countries = vec![owner(combat.id as u32)];
                    detail.sides[1].summary.countries = vec![owner(combat.id as u32 * 7)];
                    detail
                })
                .collect();
            let mut ships: Vec<_> = combats
                .iter()
                .flat_map(|combat| {
                    (0..40_u32).map(move |slot| NavalCombatShipRow {
                        combat: combat.id,
                        side: (slot % 2) as u8,
                        battle_slot: slot,
                        state: 1,
                        detection_state: i32::from(slot % 3 == 0),
                        ..Default::default()
                    })
                })
                .collect();
            ships.sort_unstable_by_key(|row| (row.combat, row.side, row.battle_slot));
            set::<tables::NavalCombats>(&mut frame, combats);
            set::<tables::NavalCombatDetails>(&mut frame, details);
            set::<tables::NavalCombatShips>(&mut frame, ships);
            let mut bases: Vec<_> = (0..2_000_u32)
                .map(|index| NavalBaseRow {
                    province: index * 6 + 1,
                    ..Default::default()
                })
                .collect();
            bases.sort_unstable_by_key(|row| row.province);
            set::<tables::NavalBases>(&mut frame, bases);
            let task_forces: Vec<_> = (0..ACTIVE * 2)
                .map(|index| TaskForceRow {
                    id: u64::from(index) + 1,
                    province: sea(index),
                    logical_owner: owner(index),
                    ..Default::default()
                })
                .collect();
            let divisions: Vec<_> = (0..ACTIVE * 20)
                .map(|index| DivisionRow {
                    id: 1_000_000 + u64::from(index),
                    province: index % PROVINCES + 1,
                    logical_owner: owner(index),
                    ..Default::default()
                })
                .collect();
            let mut orders = BTreeMap::<u32, ProvinceUnitOrderRow>::new();
            for task_force in &task_forces {
                let order = orders.entry(task_force.province).or_default();
                order.province = task_force.province;
                order.naval.push(task_force.id);
                order.mixed.push(task_force.id);
            }
            for division in &divisions {
                let order = orders.entry(division.province).or_default();
                order.province = division.province;
                order.mixed.push(division.id);
            }
            set::<tables::ProvinceUnitOrder>(&mut frame, orders.into_values().collect());
            set::<tables::TaskForces>(&mut frame, task_forces);
            set::<tables::Divisions>(&mut frame, divisions);
            let regions: BTreeMap<u32, Vec<u32>> = (1..=170)
                .map(|region| {
                    (
                        region,
                        (0..PROVINCES / 170)
                            .map(|index| index * 170 + region)
                            .collect(),
                    )
                })
                .collect();
            let masks = [
                1 << 4,
                1 << 8,
                1 << 3,
                1 << 14,
                AIR_MISSION_LAY_MINES,
                1 << 2,
            ];
            let wings = (0..5_000_u32)
                .map(|index| {
                    let region = index % 170 + 1;
                    let wing = AirWingRow {
                        id: u64::from(index) + 1,
                        owner: owner(index),
                        mission_mask: masks[index as usize % masks.len()],
                        mission_region: Some(region),
                        ..Default::default()
                    };
                    let reached = (0..16)
                        .map(|step| (index * 31 + step * 13) % PROVINCES + 1)
                        .collect();
                    (wing, inputs(reached))
                })
                .collect();
            (frame, links, regions, wings)
        }
    }

    #[test]
    fn the_initial_gate_uses_the_wings_own_operands() {
        let mut wing = AirWingRow {
            mission_mask: 4,
            mission_region: Some(90),
            mission_active: 1,
            eligibility_counter: 3,
            eligibility_limit: 3,
            ..AirWingRow::default()
        };
        assert!(passes_initial_gate(&wing));
        wing.mission_active = 0;
        assert!(!passes_initial_gate(&wing));
        wing.mission_active = 1;
        wing.eligibility_veto = 1;
        assert!(!passes_initial_gate(&wing));
        wing.eligibility_veto = 0;
        wing.eligibility_counter = 4;
        assert!(!passes_initial_gate(&wing));
        wing.eligibility_counter = 3;
        wing.mission_region = None;
        assert!(!passes_initial_gate(&wing));
        wing.mission_mask = 0x800;
        assert!(passes_initial_gate(&wing));
    }
}
