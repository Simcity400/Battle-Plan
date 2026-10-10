//! Flying aircraft scenarios from recorded simulation and content-defined rigs.
//!
//! Strategic regions are visited every two wall seconds. Eligible missions enter
//! per-region arrays: bombers mask 0x1f7fa, base equipment, fighters mask 0x85,
//! transports bit 10 and scouts bit 14. The local 50-entry limit excludes direct
//! bit-17 base-equipment appends. One set holds the viewing country and side A;
//! another holds side B. Fighter, scout and transport units divide mission-effective
//! counts by the family's `NAirGfx.*_DIVISION_FACTOR`, flooring positive counts to
//! one. Authored three-unit and one-unit steps select triple and single scenarios.
//! An unpaired bomber emits one scenario, using its own plane count to choose a
//! triple. `MAX_*_SCENARIOS` is checked by exact kind before a wing's batch, so the
//! batch can exceed it. Refusal ends a stepped family's side loop; an unpaired
//! bomber refusal skips only that wing ([`scenarios`]).
//!
//! Model selection takes the largest wing-equipment amount, keeping the first
//! tie. It tries designer and saved variant models, then equipment overrides,
//! cosmetic tag, country tag and bare key. Each prefix tries equipment name,
//! archetype name and sprite with `_entity`; graphical culture is not a rung.
//! The first registered entity wins. Refused definitions omit and count the wing
//! without trying a later candidate ([`resolve_entity`]).
//!
//! Placement uses a supplied province or a draw from the deduplicated region list.
//! Bombers and transports supply their mission's last target; scouts and side A
//! fighters supply a random reached province; side B fighters supply none.
//! Naval-patrol fighters use [`NavalPatrolSources::choose`] for their province
//! and `success`/`default` kamikaze outcome.
//! Anchors use integer map coordinates and sampled terrain height. Orientation
//! follows the designated wing's base with a signed 10..40-degree perturbation
//! from a square-root-shaped variate. Animated rig bones provide formation
//! positions, orientation, duration and initial speed.
//!
//! The base joins the animation through five-control-point nested interpolation.
//! Sampling uses distance divided by `AIRPLANES_CURVE_POINT_DENSITY`, plus three
//! points. Handle length is the smaller of 0.9 times distance and
//! `AIRPLANES_CURVE_MAX_EXTRAPOLATION`. Interior samples stay above terrain plus
//! `AIRPLANES_CURVE_MIN_ELEVATION`; travel advances by polyline arc length.
//! Take-off and landing use `AIRPLANES_SCALE_*`, with synchronized group arrivals.
//! A separate presentation clock advances while paused at
//! `AIRPLANES_ANIMATION_GLOBAL_SPEED_PER_GAMESPEED[0]`.
//!
//! Recorded simulation selects grouping, kinds, cap checks, strategic thresholds
//! and paired outcomes. Missing first-window history or selected strategic stats
//! refuse only dependent branches. CAS diversion uses the raw mission ratio.
//! Models, rigs and every define resolve through the recorded content stack.
//! Presentation clocks and seeds belong to the viewer. Deterministic random
//! removal supplies selections from unrecorded wall-clock random state. Training
//! uses the four initial bomber/fighter arrays; carrier take-off shares the
//! settled task-force route law.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

use engine::air_kamikaze::window_sum;
mod scenarios;
use installed_content::image::native::Decode;
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
        AIR_MISSION_LAY_MINES, AIR_MISSION_NAVAL_PATROL, AIR_MISSION_TRAINING,
        AIR_MISSION_ZERO_PLANE_FLAGS, AirBaseRow, AirCountryRelationRow, AirMissionInputRow,
        AirWingRow, ShipRow, TaskForceRow, tables,
    },
};
use scenarios::{RegionGroups, ScenarioWing};

use super::{
    air_participants::{AirParticipants, CountryRelations, CountryVectors, ordered_participants},
    air_reachability::{
        self, Category, Causes, MineTargetSources, MissionCountry, NavalCombatTargetSources,
        NavalOccupantSources, PortTargetSources, RegionFractions, ScoutTargetSources,
        StrategicTargetSources, TacticalTargetSources, TargetAdmission, TargetPredicates, Wars,
    },
    air_registry_sources::AirRegistrySources,
    aircraft_count::{SortieDefines, SortieSources},
    borders::HeightField,
    counters::CounterContent,
    entity_animation::{Clock, Request},
    models::{ModelField, ModelInstance, ModelLibrary},
    orders::ProvinceGeometry,
    perspective::Vision,
    tables::{Available, core, rows},
    visibility::Families,
};

pub const SEED: u32 = 0x4149_5250;

/// One scenario family's content-defined `NAirGfx` sizing: the division
/// factor, the triple threshold and the regional scenario limit.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FamilyDefines {
    factor: i32,
    triple: i32,
    limit: usize,
}

impl FamilyDefines {
    fn load(defines: &Defines, factor: &str, anim: &str, limit: &str) -> io::Result<Self> {
        let factor_value = defines.int32("NAirGfx", factor)?;
        // The builder divides by it; a negative factor floors to one unit.
        if factor_value == 0 {
            return Err(invalid_data(format!("NDefines.NAirGfx.{factor} is zero")));
        }
        Ok(Self {
            factor: factor_value,
            triple: defines.int32("NAirGfx", &format!("AIRPLANES_3_{anim}_ANIM"))?,
            // The game admits one scenario
            // below any non-positive limit.
            limit: defines.int32("NAirGfx", limit)?.max(1) as usize,
        })
    }

    /// `planes / factor`, floored to one when positive.
    fn units(self, planes: i32) -> i32 {
        if planes > 0 {
            (planes / self.factor).max(1)
        } else {
            0
        }
    }
}

/// A family whose batch loop steps by its thresholds: fighters, scouts and
/// transports. The loops never end on a non-positive step, so it is refused.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SteppedFamily {
    sizing: FamilyDefines,
    single: i32,
}

impl SteppedFamily {
    fn load(defines: &Defines, factor: &str, anim: &str, limit: &str) -> io::Result<Self> {
        let sizing = FamilyDefines::load(defines, factor, anim, limit)?;
        let single = defines.int32("NAirGfx", &format!("AIRPLANES_1_{anim}_ANIM"))?;
        if sizing.triple <= 0 || single <= 0 {
            return Err(invalid_data(format!(
                "NDefines.NAirGfx.AIRPLANES_1/3_{anim}_ANIM must be positive"
            )));
        }
        Ok(Self { sizing, single })
    }

    /// Triple scenarios while the units reach the triple threshold, else
    /// singles; the remainder is dropped.
    fn batches(self, units: i32) -> Option<(usize, usize)> {
        if units >= self.sizing.triple {
            Some((3, (units / self.sizing.triple) as usize))
        } else if units >= self.single {
            Some((1, (units / self.single) as usize))
        } else {
            None
        }
    }
}

/// The `NAirGfx` defines the flight builder and animator read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlightDefines {
    fighter: SteppedFamily,
    scout: SteppedFamily,
    /// Independent bomber sizing; CAS diversion reads `bomber_single`.
    bomber: FamilyDefines,
    bomber_single: i32,
    dogfight: SteppedFamily,
    fire_threshold: i64,
    carpet_threshold: i64,
    transport: SteppedFamily,
    /// The training pass: the wing's own planes
    /// over `TRANSPORT_DIVISION_FACTOR`, fighter-patrol kinds 0/1 at the
    /// `AIRPLANES_1/3_FIGHTER_PATROL_ANIM` steps, `MAX_TRAINING_SCENARIOS`.
    /// It compares units, not wings, so no loop is refused.
    training: SteppedFamily,
    /// `AIRPLANES_ANIMATION_GLOBAL_SPEED_PER_GAMESPEED[0]`, the paused speed
    clock_speed: f32,
    curve_density: f32,
    curve_extrapolation: f32,
    curve_elevation: f32,
    scale_takeoff: f32,
    scale_landing: f32,
    scale_min: f32,
}

impl FlightDefines {
    pub fn load(defines: &Defines) -> io::Result<Self> {
        let float = |name: &str| Ok::<_, io::Error>(defines.number("NAirGfx", name)? as f32);
        let fighter = SteppedFamily::load(
            defines,
            "FIGHTERS_DIVISION_FACTOR",
            "FIGHTER_PATROL",
            "MAX_PATROL_SCENARIOS",
        )?;
        let transport = SteppedFamily::load(
            defines,
            "TRANSPORT_DIVISION_FACTOR",
            "TRANSPORT_SUPPLY",
            "MAX_TRANSPORT_SCENARIOS",
        )?;
        let training = SteppedFamily {
            sizing: FamilyDefines {
                factor: transport.sizing.factor,
                triple: fighter.sizing.triple,
                limit: defines.int32("NAirGfx", "MAX_TRAINING_SCENARIOS")?.max(1) as usize,
            },
            single: fighter.single,
        };
        let out = Self {
            fighter,
            scout: SteppedFamily::load(
                defines,
                "SCOUT_PLANE_DIVISION_FACTOR",
                "SCOUT_PLANE_PATROL",
                "MAX_SCOUT_SCENARIOS",
            )?,
            bomber: FamilyDefines::load(
                defines,
                "BOMBERS_DIVISION_FACTOR",
                "BOMBER_BOMBING",
                "MAX_BOMBING_SCENARIOS",
            )?,
            bomber_single: defines.int32("NAirGfx", "AIRPLANES_1_BOMBER_BOMBING_ANIM")?,
            dogfight: SteppedFamily {
                sizing: FamilyDefines {
                    factor: fighter.sizing.factor,
                    triple: defines.int32("NAirGfx", "AIRPLANES_3_FIGHTER_VS_3_FIGHTER_ANIM")?,
                    limit: defines.int32("NAirGfx", "MAX_DOGFIGHTS_SCENARIOS")?.max(1) as usize,
                },
                single: defines.int32("NAirGfx", "AIRPLANES_1_FIGHTER_VS_1_FIGHTER_ANIM")?,
            },
            fire_threshold: threshold_raw(float("STRAT_BOMBER_FIREBOMB_THRESHOLD")?),
            carpet_threshold: threshold_raw(float("STRAT_BOMBER_CARPETBOMB_THRESHOLD")?),
            transport,
            training,
            clock_speed: *defines
                .numbers("NAirGfx", "AIRPLANES_ANIMATION_GLOBAL_SPEED_PER_GAMESPEED")?
                .first()
                .ok_or_else(|| {
                    invalid_data(
                        "NDefines.NAirGfx.AIRPLANES_ANIMATION_GLOBAL_SPEED_PER_GAMESPEED is empty"
                            .into(),
                    )
                })? as f32,
            curve_density: float("AIRPLANES_CURVE_POINT_DENSITY")?,
            curve_extrapolation: float("AIRPLANES_CURVE_MAX_EXTRAPOLATION")?,
            curve_elevation: float("AIRPLANES_CURVE_MIN_ELEVATION")?,
            scale_takeoff: float("AIRPLANES_SCALE_TAKEOFF_DIST")?,
            scale_landing: float("AIRPLANES_SCALE_LANDING_DIST")?,
            scale_min: float("AIRPLANES_SCALE_MIN")?,
        };
        if out.dogfight.single <= 0 || out.dogfight.sizing.triple <= 0 {
            return Err(invalid_data(
                "NAirGfx fighter pairing steps must be positive".into(),
            ));
        }
        if !(out.clock_speed.is_finite() && out.clock_speed > 0.0) {
            return Err(invalid_data(
                "NDefines.NAirGfx.AIRPLANES_ANIMATION_GLOBAL_SPEED_PER_GAMESPEED[0] non-positive playback is unsupported".into(),
            ));
        }
        // The curve's sample count divides by the density.
        if out.curve_density.is_nan() || out.curve_density <= 0.0 {
            return Err(invalid_data(
                "NDefines.NAirGfx.AIRPLANES_CURVE_POINT_DENSITY must be positive".into(),
            ));
        }
        Ok(out)
    }

    /// Scale up over the first take-off fraction. IEEE
    /// arithmetic as native, a NaN scale included.
    fn takeoff_scale(&self, t: f32) -> f32 {
        let scale = if t <= self.scale_takeoff {
            t / self.scale_takeoff
        } else {
            1.0
        };
        if scale <= self.scale_min {
            self.scale_min
        } else {
            scale
        }
    }

    /// Scale down after the landing fraction.
    fn landing_scale(&self, t: f32) -> f32 {
        let scale = if self.scale_landing <= t {
            1.0 - (t - self.scale_landing) / (1.0 - self.scale_landing)
        } else {
            1.0
        };
        if scale <= self.scale_min {
            self.scale_min
        } else {
            scale
        }
    }
}

/// The game converts an authored f32 threshold to fixed5 before comparing.
fn threshold_raw(value: f32) -> i64 {
    let rounded = value * 100_000.0 + (if value >= 0.0 { 1.0 } else { 0.0 }) - 0.5;
    // Truncating integer conversion returns the signed indefinite word on overflow or NaN.
    let limit = -(i64::MIN as f32);
    if !(-limit..limit).contains(&rounded) {
        i64::MIN
    } else {
        rounded as i64
    }
}

/// The master `DRAW_MAP_OBJECTS_CUTOFF` over every map object.
pub fn draws(camera: &super::camera::Camera) -> bool {
    camera.height < camera.defines.map_objects_cutoff
}

/// Model-selection inputs read from an equipment
/// type besides its name.
#[derive(Debug, Clone, Default)]
struct Equipment {
    /// Archetype name; the null object's is empty.
    archetype: String,
    /// The equipment type's own sprite key.
    sprite: String,
}

/// Authored track shared by every scenario of this kind, prepared once.
#[derive(Debug)]
struct Rig {
    tracks: Vec<Vec<Transform>>,
    sides: Vec<u8>,
    fps: f32,
    duration: f32,
    events: Vec<mesh::EntityEvent>,
    speed: f32,
}

#[derive(Debug)]
pub struct AircraftAssets {
    library: std::sync::RwLock<ModelLibrary>,
    roster: Arc<mesh::Roster>,
    layers: ResolvedLayers,
    /// Candidate entities the game would find that cannot be drawn here,
    /// with the reason.
    unusable: std::sync::Mutex<BTreeMap<String, String>>,
    /// Asset-load diagnostics from successful or refused lazy bakes.
    lazy_diagnostics: std::sync::Mutex<Vec<installed_content::Diagnostic>>,
    /// Equipment names by the lexer's token identity.
    keys: crate::table_access::TokenKeys,
    rigs: BTreeMap<String, Rig>,
    /// Each strategic region's provinces, from content.
    region_provinces: BTreeMap<u32, Vec<u32>>,
    /// Each region's provinces once, in first-listing order.
    region_draws: BTreeMap<u32, Vec<u32>>,
    /// Each region's land and sea fractions.
    region_fractions: BTreeMap<u32, RegionFractions>,
    province_kinds: BTreeMap<u32, ProvinceKind>,
    sortie_defines: Result<SortieDefines, String>,
    flight_defines: Result<FlightDefines, String>,
}

/// The exact suffix ladder under each available prefix. Missing recorded
/// override levels are reported, never substituted with a graphical culture.
#[cfg(test)]
pub(crate) fn candidates(tag: &str, name: &str, archetype: &str, sprite: &str) -> Vec<String> {
    super::aircraft_model::candidates("", tag, name, archetype, sprite)
}

const NOT_LOADED: &str = "its mesh did not load";

/// A name a descriptor file the parser refuses holds (`Roster::unreadable`).
/// Which of that file's blocks the engine still registers is unverified, so
/// the name may be declared there, alone or beside another declaration. The
/// roster registers a clone as the engine does, a copy of its parent, so a
/// clone of such a name is one too.
fn unreadable(roster: &mesh::Roster, name: &str) -> Option<String> {
    roster
        .unreadable_names
        .get(name)
        .map(|path| format!("may be declared in {path}, which does not parse"))
}

/// The first refused declaration a plane entity uses as
/// [`ModelLibrary::load`] bakes it: the entity, the `pdxmesh` it names, the
/// animation the mesh plays for each state it declares and each attached entity.
fn plane_refusal(
    roster: &mesh::Roster,
    entity: &str,
    ancestors: &BTreeSet<String>,
) -> Option<String> {
    let mut ancestors = ancestors.clone();
    if !ancestors.insert(entity.to_owned()) {
        return Some(format!("attachment cycle through {entity}"));
    }
    if let Some(flaw) = unreadable(roster, entity) {
        return Some(format!("it {flaw}"));
    }
    let descriptors = &roster.descriptors;
    let inherited = descriptors.inherit(entity)?;
    if let Some(state) = inherited
        .states
        .values()
        .flatten()
        .find(|state| !(state.speed.is_finite() && state.speed > 0.))
    {
        return Some(format!(
            "state {} has unsupported non-positive animation_speed {}",
            state.name, state.speed
        ));
    }
    if let Some(name) = inherited.mesh {
        if let Some(flaw) = unreadable(roster, name) {
            return Some(format!("its mesh {name} {flaw}"));
        }
        if let Some(definition) = descriptors.meshes.get(name) {
            for state in inherited.states.values().flatten() {
                if let Some((_, animation)) = definition
                    .animations
                    .iter()
                    .find(|(declared, _)| *declared == state.animation)
                    && let Some(flaw) = unreadable(roster, animation)
                {
                    return Some(format!("its animation {animation} {flaw}"));
                }
            }
        }
    }
    for attachment in &inherited.attachments {
        if let Some(reason) = plane_refusal(roster, &attachment.entity, &ancestors) {
            return Some(format!(
                "its attached entity {} is refused: {reason}",
                attachment.entity
            ));
        }
    }
    None
}

/// Why a wing's aircraft has no entity to draw.
#[derive(Debug, PartialEq)]
enum Unresolved {
    /// The wing's equipment variant is not recorded.
    Identity,
    /// No candidate for this equipment names an entity.
    Missing(String),
    /// The variant's equipment type token does not name one content key.
    Token(String),
    /// The first entity the ladder finds exists but cannot be drawn here.
    Refused { entity: String, reason: String },
}

/// Wings omitted for a refused aircraft entity, counted by entity.
#[derive(Default)]
struct Refusals(BTreeMap<(String, String), usize>);

impl Refusals {
    fn omit(&mut self, wing: u64, error: Unresolved, diagnostics: &mut Vec<String>) {
        match error {
            Unresolved::Identity => {
                diagnostics.push(format!("wing {wing} missing equipment identity"));
            }
            Unresolved::Missing(name) => {
                diagnostics.push(format!("wing {wing} unresolved aircraft equipment {name}"));
            }
            Unresolved::Token(reason) => {
                diagnostics.push(format!("wing {wing} equipment type: {reason}"));
            }
            Unresolved::Refused { entity, reason } => {
                *self.0.entry((entity, reason)).or_default() += 1
            }
        }
    }

    fn diagnostics(&self) -> impl Iterator<Item = String> + '_ {
        self.0.iter().map(|((entity, reason), count)| {
            format!("{count} wings omitted: their aircraft entity {entity} is refused: {reason}")
        })
    }
}

/// Every entity a candidate key can name that the roster knows of: declared,
/// or held by a descriptor file that does not parse, so a possibly hidden
/// declaration stops the ladder instead of vanishing.
fn declared_planes<'a>(roster: &'a mesh::Roster, keys: &BTreeSet<&str>) -> BTreeSet<&'a String> {
    roster
        .descriptors
        .entities
        .keys()
        .chain(roster.unreadable_names.keys())
        .filter(|entity| {
            entity.strip_suffix("_entity").is_some_and(|stem| {
                keys.contains(stem)
                    || stem
                        .match_indices('_')
                        .any(|(index, _)| keys.contains(&stem[index + 1..]))
            })
        })
        .collect()
}

/// Bakes the declared plane entities whose declarations, and the
/// declarations they use, the engine's choice for is verified
/// ([`plane_refusal`]). A refused entity still exists for the engine's name
/// lookup, so it is kept with its reason to stop resolution rather than
/// being skipped, and so is one whose mesh does not load. When every
/// candidate is refused the library is empty and every reason is kept.
fn plane_library<'a>(
    layers: &ResolvedLayers,
    decode: Decode,
    roster: &mesh::Roster,
    declared: impl IntoIterator<Item = &'a String>,
    warnings: &mut Vec<installed_content::Diagnostic>,
) -> io::Result<(ModelLibrary, BTreeMap<String, String>)> {
    let mut unusable = BTreeMap::new();
    let mut names = Vec::new();
    for entity in declared {
        match plane_refusal(roster, entity, &BTreeSet::new()) {
            Some(reason) => {
                unusable.insert(entity.clone(), reason);
            }
            None => names.push(entity.clone()),
        }
    }
    if !unusable.is_empty() {
        warnings.push((installed_content::DiagnosticKind::Refused, format!(
            "Aircraft entities are refused, and wings whose aircraft resolves to one are not drawn: {}",
            unusable
                .iter()
                .map(|(entity, reason)| format!("{entity} ({reason})"))
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    let library = ModelLibrary::load(layers, decode, &roster.descriptors, &names, warnings)?;
    for name in names {
        if library.slot(&name).is_none() {
            unusable.insert(name, NOT_LOADED.to_owned());
        }
    }
    Ok((library, unusable))
}

/// The first candidate the engine's name lookup finds, `None` when none
/// exists. A refused entity ends the ladder as a drawn one would; it does
/// not fall through to a lower-priority candidate.
fn resolve_entity(
    candidates: &[String],
    slot: impl Fn(&str) -> Option<usize>,
    unusable: &BTreeMap<String, String>,
) -> Result<Option<usize>, Unresolved> {
    for entity in candidates {
        if let Some(slot) = slot(entity) {
            return Ok(Some(slot));
        }
        if let Some(reason) = unusable.get(entity) {
            return Err(Unresolved::Refused {
                entity: entity.clone(),
                reason: reason.clone(),
            });
        }
    }
    Ok(None)
}

/// A scenario entity's mesh and the animation of its `default` state; the
/// scenario entities name that state, not the implicit `idle`.
fn scenario<'a>(
    descriptors: &'a mesh::Descriptors,
    entity: &str,
    state: &str,
) -> Option<(&'a mesh::MeshDefinition, &'a mesh::AnimationDefinition, f32)> {
    let inherited = descriptors.inherit(entity)?;
    let definition = descriptors.meshes.get(inherited.mesh?)?;
    let state = inherited.states.get(state)?.first()?;
    let (_, name) = definition
        .animations
        .iter()
        .find(|(declared, _)| *declared == state.animation)?;
    Some((definition, descriptors.animations.get(name)?, state.speed))
}

/// The first refused declaration a scenario rig uses: the scenario entity,
/// its mesh and the animation of the state it plays.
fn scenario_refusal(roster: &mesh::Roster, entity: &str, state: &str) -> Option<String> {
    if let Some(flaw) = unreadable(roster, entity) {
        return Some(format!("it {flaw}"));
    }
    let descriptors = &roster.descriptors;
    let inherited = descriptors.inherit(entity)?;
    let mesh = inherited.mesh?;
    if let Some(flaw) = unreadable(roster, mesh) {
        return Some(format!("its mesh {mesh} {flaw}"));
    }
    let state = inherited.states.get(state)?.first()?;
    let (_, animation) = descriptors
        .meshes
        .get(mesh)?
        .animations
        .iter()
        .find(|(declared, _)| *declared == state.animation)?;
    unreadable(roster, animation).map(|flaw| format!("its animation {animation} {flaw}"))
}

/// The scenario outcomes the game plays: 0 `default`,
/// 1 `pass`, 2 `success`, 3 `fail`.
const SUCCESS: u8 = 2;

fn state_name(outcome: u8) -> &'static str {
    match outcome {
        1 => "pass",
        2 => "success",
        3 => "fail",
        _ => "default",
    }
}

/// A rig's key: the scenario name, with the outcome's state when not default.
fn rig_key(scenario: &str, outcome: u8) -> String {
    if outcome != 0 {
        format!("{scenario}_{}", state_name(outcome))
    } else {
        scenario.to_owned()
    }
}

/// Scenario names, each resolved through recorded content.
fn scenario_name(kind: u8) -> &'static str {
    match kind {
        0 => "fighter_1_patrol",
        1 => "fighter_3_patrol",
        2 => "scout_1_patrol",
        3 => "scout_3_patrol",
        4 => "bomber_1_CAS",
        5 => "bomber_3_CAS",
        6 => "bomber_1_bombing",
        7 => "bomber_3_bombing",
        8 => "bomber_1_carpetbombing",
        9 => "bomber_3_carpetbombing",
        10 => "bomber_1_firebombing",
        11 => "bomber_3_firebombing",
        12 => "fighter_1_vs_bomber_1",
        13 => "fighter_1_vs_fighter_1",
        14 => "fighter_3_vs_fighter_3",
        16 => "fighter_1_kamikaze",
        17 => "fighter_3_kamikaze",
        18 => "fighter_1_vs_transport_1",
        19 => "transport_1_supplying",
        20 => "transport_3_supplying",
        _ => unreachable!("conventional aircraft scenario kind"),
    }
}

/// Side-local attachment order for the scenario rig.
fn scenario_bones(kind: u8) -> Vec<(u8, String)> {
    let single = |bone: &str| vec![(0, bone.to_owned())];
    let triple = |bone: &str| (1..=3).map(|n| (0, format!("{bone}_{n}"))).collect();
    match kind {
        0 | 2 | 4 => single("fighter"),
        1 | 3 | 5 | 17 => triple("fighter"),
        6 | 8 | 10 | 19 => single("bomber"),
        7 | 9 | 11 | 20 => triple("bomber"),
        12 | 18 => vec![(0, "fighter".into()), (1, "bomber".into())],
        13 => vec![(0, "fighter1".into()), (1, "fighter2".into())],
        14 => (1..=3)
            .map(|n| (0, format!("fighter_{n}")))
            .chain((1..=3).map(|n| (1, format!("defender_{n}"))))
            .collect(),
        16 => single("fighter_1"),
        _ => unreachable!("conventional aircraft scenario kind"),
    }
}

/// Resolve each locator once, then
/// sample the shared native source-order palette. Animation joint names do
/// not rebind the skeleton; no content-specific alias is needed.
fn scenario_tracks(
    source: &super::mesh_pose::Source,
    animation: &mesh::Animation,
    bones: &[(u8, String)],
) -> io::Result<Vec<Vec<Transform>>> {
    use super::mesh_pose::{self, LookupMode};
    let bindings = bones
        .iter()
        .map(|(_, bone)| {
            mesh_pose::lookup(source, bone, LookupMode::Naval)
                .ok_or_else(|| invalid_data(format!("attachment node {bone} is absent")))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let mut tracks = vec![Vec::with_capacity(animation.samples as usize); bindings.len()];
    for frame in 0..animation.samples {
        let pose = mesh_pose::assemble(
            &source.skeleton,
            &animation.frame_trs(frame),
            source.mesh_scale,
            false,
        )?;
        for (track, binding) in tracks.iter_mut().zip(&bindings) {
            let local = binding
                .local(Ok(Some(&pose.joints)))
                .map_err(invalid_data)?;
            let matrix = mesh_pose::multiply(mesh_pose::scale(source.entity_scale), local);
            track.push(Transform {
                translation: [matrix[3][0], matrix[3][1], matrix[3][2]],
                basis: std::array::from_fn(|column| {
                    [matrix[column][0], matrix[column][1], matrix[column][2]]
                }),
            });
        }
    }
    Ok(tracks)
}

impl AircraftAssets {
    pub(crate) fn lazy_diagnostics(&self) -> Vec<installed_content::Diagnostic> {
        self.lazy_diagnostics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// What the prepared content refuses when a flight meets it: entities
    /// that cannot be drawn, equipment names one token under the lexer's
    /// fold, and the sortie and flight defines.
    pub(crate) fn refusals(&self) -> Vec<String> {
        let mut refusals: Vec<String> = self
            .unusable
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(entity, why)| format!("entity {entity}: {why}"))
            .collect();
        refusals.extend(self.keys.ambiguous().map(|keys| {
            format!(
                "equipment {} are one token under the lexer's fold",
                keys.join(", ")
            )
        }));
        for why in [
            self.sortie_defines.as_ref().err(),
            self.flight_defines.as_ref().err(),
        ]
        .into_iter()
        .flatten()
        {
            refusals.push(why.clone());
        }
        refusals.extend(
            self.lazy_diagnostics()
                .into_iter()
                .filter(|(kind, _)| *kind == installed_content::DiagnosticKind::Refused)
                .map(|(_, reason)| reason),
        );
        refusals
    }

    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        map: &MapData,
        defines: &Defines,
        regions: &BTreeMap<u32, StrategicRegion>,
        warnings: &mut Vec<installed_content::Diagnostic>,
    ) -> io::Result<Self> {
        // The content's equipment database, `duplicate_archetypes` copies
        // included: the archetype is resolved as the definitions are read
        // (a `parent` passes its archetype on), and an `archetype` read
        // copies the archetype's sprite while the type's own is empty
        // A duplicate's own sprite replaces its copies'
        let database = installed_content::equipment::Database::load_complete(layers)?;
        let text = |bytes: &[u8], what: &str| {
            installed_content::parser::text(bytes)
                .map(str::to_owned)
                .map_err(|refused| invalid_data(format!("equipment {what}: {refused}")))
        };
        let mut equipment = BTreeMap::new();
        for kind in &database.equipment {
            let name = text(&kind.name, "name")?;
            let archetype = match kind.archetype {
                Some(archetype) => text(&database.equipment[archetype].name, "name")?,
                None => String::new(),
            };
            let sprite = text(&kind.sprite, &format!("{name} sprite"))?;
            equipment.insert(name, Equipment { archetype, sprite });
        }
        let roster = layers.descriptor_roster()?;
        let descriptors = &roster.descriptors;
        let mut rigs = BTreeMap::new();
        // Scenario and outcome catalogs provide authored
        // independent and paired rigs, including both callback sides.
        let mut scenarios = Vec::new();
        for kind in (0..=14).chain(16..=20) {
            let outcomes: &[u8] = match kind {
                12 | 18 => &[1, 2],
                13 | 14 => &[1, 2, 3],
                16 | 17 => &[0, SUCCESS],
                _ => &[0],
            };
            for &outcome in outcomes {
                scenarios.push((
                    scenario_name(kind).to_owned(),
                    scenario_bones(kind),
                    outcome,
                ));
            }
        }
        for (key, bones, outcome) in scenarios {
            let entity = format!("{key}_entity");
            // The engine's choice between duplicate or out-of-order
            // declarations is unverified, so a scenario using one is refused.
            if let Some(reason) = scenario_refusal(&roster, &entity, state_name(outcome)) {
                warnings.push((
                    installed_content::DiagnosticKind::Refused,
                    format!("Aircraft scenario {entity} is refused and not flown: {reason}"),
                ));
                continue;
            }
            let Some((definition, animation, speed)) =
                scenario(descriptors, &entity, state_name(outcome))
            else {
                continue;
            };
            let mesh_bytes = layers.read_uncovered(&definition.file)?;
            let tree = mesh::parse(&mesh_bytes)?;
            let skeleton = mesh::native_skeleton(&tree, definition.scale)?;
            let source = super::mesh_pose::Source {
                pdxmesh: true,
                binary_locators: super::mesh_pose::binary_locators(
                    &tree,
                    &skeleton,
                    definition.scale,
                )?,
                // Registration keeps duplicate names in order and the cloned
                // parent's mesh identity.
                entity_locators: super::mesh_pose::entity_locators(
                    &descriptors.entities[&entity].locators,
                    &skeleton,
                    Some(&definition.name),
                ),
                skeleton,
                mesh_scale: definition.scale,
                entity_scale: descriptors.resolve_entity(&entity).unwrap().scale,
                frozen: None,
                game_data: None,
            };
            let anim = mesh::read_animation(&layers.read_uncovered(&animation.file)?)?;
            let tracks = match scenario_tracks(&source, &anim, &bones) {
                Ok(tracks) if anim.samples >= 2 => tracks,
                Ok(_) => {
                    warnings.push((installed_content::DiagnosticKind::Refused,
                        format!("Aircraft scenario {entity} has fewer than two animation samples; skipped")));
                    continue;
                }
                Err(reason) => {
                    warnings.push((
                        installed_content::DiagnosticKind::Refused,
                        format!("Aircraft scenario {entity} attachment tracks refused: {reason}"),
                    ));
                    continue;
                }
            };
            // The state's `animation_speed` plays the track faster.
            if !(speed.is_finite() && speed > 0.0) {
                warnings.push((
                    installed_content::DiagnosticKind::Refused,
                    format!(
                        "Aircraft scenario {entity} has a non-positive animation_speed; skipped"
                    ),
                ));
                continue;
            }
            let fps = anim.fps * speed;
            rigs.insert(
                rig_key(&key, outcome),
                Rig {
                    tracks,
                    sides: bones.iter().map(|(side, _)| *side).collect(),
                    fps,
                    duration: (anim.samples - 1) as f32 / fps,
                    events: descriptors.inherit(&entity).unwrap().states[state_name(outcome)][0]
                        .events
                        .clone(),
                    speed,
                },
            );
        }
        // Entities any candidate key can name: `<key>_entity` or
        // `<prefix>_<key>_entity` for an equipment, archetype or sprite key.
        let keys: BTreeSet<&str> = equipment
            .iter()
            .flat_map(|(name, eq)| [name.as_str(), eq.archetype.as_str(), eq.sprite.as_str()])
            .filter(|key| !key.is_empty())
            .collect();
        let declared = declared_planes(&roster, &keys);
        let (library, unusable) = plane_library(layers, decode, &roster, declared, warnings)?;
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
        let region_draws = region_provinces
            .iter()
            .map(|(&region, provinces)| (region, first_listings(provinces)))
            .collect();
        Ok(Self {
            library: std::sync::RwLock::new(library),
            roster: Arc::clone(&roster),
            layers: layers.clone(),
            unusable: std::sync::Mutex::new(unusable),
            lazy_diagnostics: Default::default(),
            keys: crate::table_access::TokenKeys::new(equipment.keys()),
            rigs,
            region_provinces,
            region_draws,
            region_fractions,
            province_kinds,
            sortie_defines: SortieDefines::load(defines).map_err(|error| error.to_string()),
            flight_defines: FlightDefines::load(defines).map_err(|error| error.to_string()),
        })
    }

    pub fn library(&self) -> std::sync::RwLockReadGuard<'_, ModelLibrary> {
        self.library
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Selected designer/explicit/graphic names need not have an equipment
    /// suffix. Bake them from the same authenticated roster on first use;
    /// a failed selected entity is remembered and never replaced.
    fn resolve(&self, entity: &str) -> Result<usize, Unresolved> {
        let mut unusable = self
            .unusable
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(reason) = unusable.get(entity) {
            return Err(Unresolved::Refused {
                entity: entity.to_owned(),
                reason: reason.clone(),
            });
        }
        if let Some(slot) = self.library().slot(entity) {
            return Ok(slot);
        }
        let name = entity.to_owned();
        let mut warnings = Vec::new();
        let baked = plane_library(
            &self.layers,
            self.library().decode(),
            &self.roster,
            [&name],
            &mut warnings,
        );
        // Usable actors can still omit attachments or audio.
        // Keep the loader's diagnostic kinds, names and occurrence counts.
        self.lazy_diagnostics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend(warnings);
        let (shard, refused) = baked.map_err(|error| Unresolved::Refused {
            entity: name.clone(),
            reason: error.to_string(),
        })?;
        unusable.extend(refused);
        self.library
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .merge(shard);
        resolve_entity(
            std::slice::from_ref(&name),
            |entity| self.library().slot(entity),
            &unusable,
        )?
        .ok_or(Unresolved::Missing(name))
    }
}

/// Each province once, in first listing order.
fn first_listings(provinces: &[u32]) -> Vec<u32> {
    let mut seen = BTreeSet::new();
    provinces
        .iter()
        .copied()
        .filter(|province| seen.insert(*province))
        .collect()
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
    /// Five-point nested interpolation (not a cubic
    /// Bezier and not the land counter's Catmull-Rom spline).
    fn build(
        start: [f32; 3],
        end: [f32; 3],
        back: [f32; 3],
        takeoff: bool,
        heights: &HeightField<'_>,
        defines: &FlightDefines,
    ) -> Self {
        let distance = norm(sub(end, start));
        // The game scales in f64 by the f32 0.9 factor, then caps.
        let h = (f64::from(distance) * f64::from(0.9_f32)) as f32;
        let h = if defines.curve_extrapolation <= h {
            defines.curve_extrapolation
        } else {
            h
        };
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
        let count = ((f64::from(distance) / f64::from(defines.curve_density)) as i32)
            .saturating_add(3)
            .max(2) as usize;
        let mut points = Vec::with_capacity(count);
        let mut arcs = Vec::with_capacity(count);
        for i in 0..count {
            let t = i as f32 / (count - 1) as f32;
            let mut p = lerp(start, lerp(p1, lerp(p2, lerp(p3, end, t), t), t), t);
            if i > 0 && i + 1 < count {
                p[1] = p[1].max(heights.height_at_world(p[0], p[2]) + defines.curve_elevation);
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
    requests: Arc<Vec<Request>>,
}

#[derive(Clone, Default)]
pub struct AircraftPlan {
    flights: Vec<Flight>,
    pub diagnostics: Vec<String>,
}

type RegionMembership = (BTreeMap<u32, Vec<u32>>, BTreeMap<u32, RegionFractions>);

/// The game skips a listed ID the province
/// table does not hold and counts every other listing, duplicates included,
/// in the region's total and its land (kind bit 0) or sea (bits 0 and 1
/// clear) count. The skips are counted in one warning.
/// A_/d_ choose the attacking/defending array; without a
/// prefix both arrays receive the callback. A digit chooses a one-based
/// member; zero or an out-of-range number broadcasts. Native skips exactly
/// two bytes after a numeric prefix, even for a multi-digit number.
fn callback(id: &str, side: u8, track: usize, count: usize) -> Option<u32> {
    let bytes = id.as_bytes();
    let rest = if bytes.starts_with(b"a_") {
        if side != 0 {
            return None;
        }
        &bytes[2..]
    } else if bytes.starts_with(b"d_") {
        if side != 1 {
            return None;
        }
        &bytes[2..]
    } else {
        bytes
    };
    let state = if rest.first().is_some_and(u8::is_ascii_digit) {
        let number = installed_content::fixed5::decimal_i32(rest);
        if number > 0 && number as usize <= count && number as usize != track + 1 {
            return None;
        }
        rest.get(2..).unwrap_or_default()
    } else {
        rest
    };
    Some(
        state
            .iter()
            .take_while(|byte| **byte != 0)
            .fold(0x811c_9dc5u32, |hash, byte| {
                (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
            }),
    )
}

#[cfg(test)]
pub(crate) fn scenario_requests(
    events: &[mesh::EntityEvent],
    speed: f32,
    actor: &super::entity_animation::Rig,
    count: usize,
    track: usize,
) -> Vec<Request> {
    scenario_side_requests(events, speed, actor, 0, count, track)
}

fn scenario_side_requests(
    events: &[mesh::EntityEvent],
    speed: f32,
    actor: &super::entity_animation::Rig,
    side: u8,
    count: usize,
    track: usize,
) -> Vec<Request> {
    let mut requests = Vec::new();
    for event in events {
        let Some(hash) = event
            .id
            .as_deref()
            .and_then(|id| callback(id, side, track, count))
        else {
            continue;
        };
        let Some(state) = actor.nodes[0]
            .clips
            .iter()
            .map(|clip| &clip.state.name)
            .find(|name| {
                name.bytes().fold(0x811c_9dc5u32, |hash, byte| {
                    (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
                }) == hash
            })
        else {
            continue;
        };
        requests.push(Request {
            time: event.time / speed,
            phase_time: None,
            state: state.clone(),
        });
    }
    requests.sort_by(|a, b| a.time.total_cmp(&b.time));
    requests
}

fn flight_requests(
    flight: &Flight,
    rig: &Rig,
    actor: &super::entity_animation::Rig,
    rate: f32,
) -> Vec<Request> {
    let side = rig.sides[flight.track];
    scenario_side_requests(
        &rig.events,
        rig.speed,
        actor,
        side,
        rig.sides.iter().filter(|&&member| member == side).count(),
        rig.sides[..flight.track]
            .iter()
            .filter(|&&member| member == side)
            .count(),
    )
    .into_iter()
    .filter(|request| request.time <= rig.duration)
    .map(|request| Request {
        time: (flight.group_travel + request.time - flight.delay) / rate,
        ..request
    })
    .collect()
}

fn region_membership(
    regions: impl Iterator<Item = (u32, Vec<u32>)>,
    kinds: &BTreeMap<u32, ProvinceKind>,
    warnings: &mut Vec<installed_content::Diagnostic>,
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
        warnings.push((installed_content::DiagnosticKind::Warning, format!(
            "strategic regions list {skipped} provinces missing from the map definitions; the game skips them"
        )));
    }
    (members, fractions)
}

/// This hour's target scans, built once and shared by every wing,
/// with the content fractions, the DLC mask and the mine cap they meet.
struct Scans<'a> {
    tactical: Option<TacticalTargetSources<'a>>,
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
    /// Target-admission chain for a wing over its mission region.
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
        if mask & (1 << 1) != 0 {
            targets.tactical = self
                .tactical
                .as_ref()
                .and_then(|scan| scan.admit(wing, country, reached));
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

/// Direct rejections before target scans: no mission, the veto,
/// the counter over its limit, or an inactive regional mission (training
/// flies without one).
fn passes_initial_gate(wing: &AirWingRow) -> bool {
    wing.mission_mask != 0
        && wing.eligibility_veto == 0
        && wing.eligibility_counter <= wing.eligibility_limit
        && ((wing.mission_region.is_some() && wing.mission_active != 0)
            || wing.mission_mask & 0x800 != 0)
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

fn invalid_data(reason: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

/// the game's two arrays for the viewing country. Without complete
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

/// One registered wing in the game's visit order.
#[derive(Clone, Copy)]
struct Visit<'a> {
    /// Which grouping call collects it: the
    /// viewing country, then one per participant entry.
    call: usize,
    /// Its group arrays: 0 for the viewing country and array A, 1 for B.
    side: usize,
    /// The viewing country's own call, the only one given a scout array.
    own: bool,
    region: u32,
    wing: &'a AirWingRow,
}

/// Registered wings in the game's visit order: region IDs ascending, then
/// the viewing country's wings and those of arrays A and B, each country's in
/// registration order. A country in both arrays is visited twice, as natively.
fn visit_order<'a>(
    wings: &'a [AirWingRow],
    registrations: &[replay::rows::AirRegionalMissionRow],
    links: &BTreeMap<u32, u32>,
    ours: u32,
    participants: &AirParticipants,
) -> io::Result<Vec<Visit<'a>>> {
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
    let mut call = 0;
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
        let countries = std::iter::once((ours, 0))
            .chain(participants.a.iter().map(|raw| (*raw, 0)))
            .chain(participants.b.iter().map(|raw| (*raw, 1)));
        for (index, (raw, side)) in countries.enumerate() {
            let country = if index == 0 {
                Some(raw)
            } else {
                links.get(&raw).copied()
            };
            if let Some(wings) = country.and_then(|country| by_country.get(&country)) {
                ordered.extend(wings.iter().map(|&(_, wing)| Visit {
                    call,
                    side,
                    own: index == 0,
                    region,
                    wing,
                }));
            }
            call += 1;
        }
    }
    Ok(ordered)
}

/// the game's per-region arrays: bombers,
/// fighters, base equipment, transports, scouts. Direct bit17 appends bypass
/// their local cap; the grouping call's coarse all-full gate still applies.
const GROUP_CAPACITY: usize = 50;

#[derive(Default)]
struct Groups {
    region: Option<u32>,
    call: Option<usize>,
    open: bool,
    counts: [[usize; 5]; 2],
}

impl Groups {
    /// Whether an eligible wing enters its group. A call is skipped whole
    /// when its side's first four groups are full.
    fn admit(&mut self, visit: &Visit<'_>, group: usize, last_target: i32) -> bool {
        if self.region != Some(visit.region) {
            *self = Self {
                region: Some(visit.region),
                ..Self::default()
            };
        }
        let counts = &mut self.counts[visit.side];
        if self.call != Some(visit.call) {
            self.call = Some(visit.call);
            self.open = counts[..4].iter().any(|&count| count < GROUP_CAPACITY);
        }
        // Direct bit-17 appends have no local 50-entry cap. The
        // earlier bomber-definition arm still has its cap when masks overlap.
        let mask = visit.wing.mission_mask;
        let uncapped = group == 2
            && mask & 0x20000 != 0
            && (mask & 0x1f7fa == 0 || mask & 0x80 != 0 || last_target == 0);
        if !self.open || (!uncapped && counts[group] >= GROUP_CAPACITY) {
            return false;
        }
        counts[group] += 1;
        true
    }
}

/// Recorded naval-patrol inputs: each country's task forces in a naval
/// combat as (province, stored region), in fleet order, and each country's
/// window-0 history sums per region.
struct NavalPatrolSources {
    combats: BTreeMap<u32, Vec<(u32, u32)>>,
    sums: BTreeMap<(u32, u32), (i64, i64)>,
}

impl NavalPatrolSources {
    /// `None` without both complete tables.
    fn read(frame: &DecodedFrame) -> io::Result<Option<Self>> {
        let (Available::Complete(combats), Available::Complete(histories)) = (
            rows::<tables::AirNavalPatrolCombats>(frame)?,
            rows::<tables::AirNavalPatrolHistories>(frame)?,
        ) else {
            return Ok(None);
        };
        let mut by_country = BTreeMap::<u32, Vec<(u32, (u32, u32))>>::new();
        for row in combats {
            by_country
                .entry(row.country)
                .or_default()
                .push((row.ordinal, (row.province, row.region)));
        }
        let mut combats = BTreeMap::new();
        for (country, mut rows) in by_country {
            rows.sort_unstable();
            if rows
                .iter()
                .enumerate()
                .any(|(index, row)| row.0 as usize != index)
            {
                return Err(invalid(
                    "naval-patrol task-force ordinals are not contiguous",
                ));
            }
            combats.insert(
                country,
                rows.into_iter().map(|(_, combat)| combat).collect(),
            );
        }
        let mut entries = BTreeMap::<(u32, u32), Vec<(u32, i64, i64)>>::new();
        for row in histories {
            if row.is_availability_witness() {
                continue;
            }
            entries.entry((row.country, row.region)).or_default().push((
                row.index,
                row.enemy_ships_sunk_raw,
                row.our_fighters_lost_raw,
            ));
        }
        let mut sums = BTreeMap::new();
        for (key, mut rows) in entries {
            rows.sort_unstable();
            let sunk: Vec<_> = rows.iter().map(|row| row.1).collect();
            let lost: Vec<_> = rows.iter().map(|row| row.2).collect();
            sums.insert(key, (window_sum(&sunk), window_sum(&lost)));
        }
        Ok(Some(Self { combats, sums }))
    }

    /// The outcome and province the game hands the builder. The first of
    /// the wing country's task forces fighting a naval combat in the mission
    /// region decides it, through the viewer's history there: enemy ships
    /// sunk and our fighters lost, each in whole units. Both positive: the
    /// combat's province, `success`. None sunk and a fighter lost: a random
    /// reached sea province other than the combat's,
    /// `success`. Otherwise, or with no such task force or history, a random
    /// reached province, `default`.
    fn choose(
        &self,
        country: u32,
        region: u32,
        viewer: u32,
        reached: &[u32],
        rng: &mut Random,
        kinds: &BTreeMap<u32, ProvinceKind>,
    ) -> (u8, u32) {
        let start = rng.next() as usize % reached.len();
        let fallback = (0, reached[start]);
        let Some(combat) = self
            .combats
            .get(&country)
            .into_iter()
            .flatten()
            .find(|combat| combat.1 == region)
            .map(|combat| combat.0)
        else {
            return fallback;
        };
        let Some(&(sunk, lost)) = self.sums.get(&(viewer, region)) else {
            return fallback;
        };
        let (sunk, lost) = ((sunk / 100_000) as i32, (lost / 100_000) as i32);
        if sunk > 0 {
            return if lost > 0 {
                (SUCCESS, combat)
            } else {
                fallback
            };
        }
        if sunk != 0 || lost < 1 {
            return fallback;
        }
        // Scan forward from the draw for a sea province other than the
        // combat's; after a full turn the draw itself stands.
        let province = (0..reached.len())
            .map(|step| reached[(start + step) % reached.len()])
            .find(|&province| {
                province != combat && kinds.get(&province) == Some(&ProvinceKind::Sea)
            })
            .unwrap_or(reached[start]);
        (SUCCESS, province)
    }
}

/// A stateless base's take-off point: its host ship's task force
/// resolved through its host ship and drawn where the task force's
/// map animator stands.
struct CarrierSources<'a> {
    ships: Option<&'a [ShipRow]>,
    task_forces: Option<&'a [TaskForceRow]>,
    registry: &'a AirRegistrySources,
    origin: &'a dyn Fn(&TaskForceRow) -> Result<[f32; 3], &'static str>,
}

impl<'a> CarrierSources<'a> {
    fn read(
        frame: &'a DecodedFrame,
        registry: &'a AirRegistrySources,
        origin: &'a dyn Fn(&TaskForceRow) -> Result<[f32; 3], &'static str>,
    ) -> io::Result<Self> {
        let ships = match rows::<tables::Ships>(frame)? {
            Available::Complete(rows) => Some(rows),
            _ => None,
        };
        let task_forces = match rows::<tables::TaskForces>(frame)? {
            Available::Complete(rows) => Some(rows),
            _ => None,
        };
        Ok(Self {
            ships,
            task_forces,
            registry,
            origin,
        })
    }

    /// The host ship's task force.
    fn task_force(&self, base: &AirBaseRow) -> Result<&'a TaskForceRow, &'static str> {
        if base.host == 0 {
            return Err("the stateless base has no host");
        }
        let ships = self.ships.ok_or("the ship census is incomplete")?;
        let ship = ships
            .binary_search_by_key(&base.host, |row| row.id)
            .map(|index| &ships[index])
            .map_err(|_| "the carrier is outside the recorded ship roster")?;
        let task_forces = self
            .task_forces
            .ok_or("the task-force census is incomplete")?;
        task_forces
            .binary_search_by_key(&ship.task_force, |row| row.id)
            .map(|index| &task_forces[index])
            .map_err(|_| "the carrier's task force is outside the recorded roster")
    }

    /// Province lookup for a stateless base: its host's
    /// task-force location, which the game reads without a null check.
    fn province(&self, base: &AirBaseRow) -> Result<u32, &'static str> {
        if self.registry.hosts_without_task_force.contains(&base.id) {
            return Err("the carrier has no task force");
        }
        match self.task_force(base)?.province {
            0 => Err("the carrier's task force has no province"),
            province => Ok(province),
        }
    }

    fn start(&self, base: &AirBaseRow) -> Result<[f32; 3], &'static str> {
        if self.registry.hosts_without_task_force.contains(&base.id) {
            return Err("the carrier has no task force");
        }
        let task_force = self.task_force(base)?;
        let owner = self
            .registry
            .host_owners
            .get(&base.id)
            .ok_or("the carrier registry host does not resolve")?;
        if *owner != task_force.roster_owner {
            return Err("the carrier's registry and fleet owners disagree");
        }
        (self.origin)(task_force)
    }
}

/// The game counts exact kind and outcome
/// before a batch; a complete admitted batch can exceed the content limit.
#[derive(Default)]
struct Caps {
    counts: BTreeMap<(u32, String), usize>,
}

/// the game's arrays: bombers, fighters, base
/// equipment, transports, scouts.
fn group_index(category: Category) -> usize {
    match category {
        Category::BomberFlight => 0,
        Category::Fighter => 1,
        Category::BaseEquipment => 2,
        Category::Transport => 3,
        Category::Scout => 4,
    }
}

/// A positive override, else
/// the pool owner.
fn effective_country(wing: &AirWingRow) -> u32 {
    if wing.country_override > 0 {
        wing.country_override as u32
    } else {
        wing.owner
    }
}

/// The seed of a wing's one draw before its batches (its province).
fn wing_random(wing: u64, region: u32) -> Random {
    Random::new(SEED ^ wing as u32 ^ (wing >> 32) as u32 ^ region.rotate_left(16) ^ 0xffff)
}

/// Where a wing's scenarios take off: its state's authored air-base slot, or
/// its carrier.
fn take_off(
    content: &CounterContent,
    carriers: &CarrierSources<'_>,
    base: &AirBaseRow,
) -> Result<[f32; 3], &'static str> {
    match base.state {
        Some(state) => content
            .air_base(state)
            .ok_or("its state has no authored air-base slot"),
        None => carriers.start(base),
    }
}

/// The map a scenario is placed on.
struct Placement<'a, 'h> {
    geometry: &'a ProvinceGeometry,
    heights: &'a HeightField<'h>,
    defines: &'a FlightDefines,
}

/// What every scenario of one wing's batch shares.
struct Batch<'a> {
    wing: u64,
    region: u32,
    key: &'a str,
    rig: &'a Rig,
    /// The game obtains orientation from the designated wing's base,
    /// independently of whether the game can create its aircraft entity.
    start: [f32; 3],
    attacker: Option<Actor>,
    defender: Option<Actor>,
}

/// First-window scenario sums, in channel order 10/17/3/51/58/44/24/65.
/// A captured empty window is zero; an older table without its acquisition
/// witness cannot decide the paired outcomes.
struct ScenarioHistory {
    sums: BTreeMap<u32, [i64; 8]>,
    unavailable: BTreeMap<u32, &'static str>,
}

impl ScenarioHistory {
    fn read(frame: &DecodedFrame, viewer: u32) -> io::Result<Result<Self, &'static str>> {
        let Available::Complete(histories) = rows::<tables::AirNavalPatrolHistories>(frame)? else {
            return Ok(Err("first-window aircraft history census incomplete"));
        };
        let records: Vec<_> = histories
            .iter()
            .filter(|row| row.country == viewer)
            .collect();
        if !records.iter().any(|row| {
            row.is_availability_witness()
                && row.scenario_channels == replay::rows::Recorded::Value(Some([0; 7]))
                && row.enemy_ships_sunk_raw == 0
                && row.our_fighters_lost_raw == 0
        }) {
            return Ok(Err(
                "first-window aircraft scenario channels were not recorded",
            ));
        }
        let mut regions = BTreeMap::<u32, Vec<(u32, [i64; 8])>>::new();
        let mut unavailable = BTreeMap::new();
        for row in records {
            if row.is_availability_witness() {
                continue;
            }
            let [a, b, c, d, e, f, g] = match row.scenario_channels {
                replay::rows::Recorded::Value(Some(values)) => values,
                replay::rows::Recorded::Value(None) => {
                    unavailable
                        .entry(row.region)
                        .or_insert("first-window aircraft scenario channels unavailable");
                    continue;
                }
                replay::rows::Recorded::NotRecorded => {
                    unavailable
                        .entry(row.region)
                        .or_insert("first-window aircraft scenario channels were not recorded");
                    continue;
                }
            };
            regions
                .entry(row.region)
                .or_default()
                .push((row.index, [a, b, row.our_fighters_lost_raw, c, d, e, f, g]));
        }
        let sums = regions
            .into_iter()
            .map(|(region, mut records)| {
                records.sort_by_key(|record| record.0);
                let sums = std::array::from_fn(|channel| {
                    let words: Vec<_> = records.iter().map(|record| record.1[channel]).collect();
                    window_sum(&words)
                });
                (region, sums)
            })
            .collect();
        Ok(Ok(Self { sums, unavailable }))
    }

    fn region(&self, region: u32) -> Result<[i64; 8], &'static str> {
        match self.unavailable.get(&region) {
            Some(reason) => Err(*reason),
            None => Ok(self.sums.get(&region).copied().unwrap_or([0; 8])),
        }
    }
}

#[derive(Clone, Copy)]
struct Actor {
    mesh: usize,
    wing: u64,
    start: [f32; 3],
}

impl Batch<'_> {
    /// The seed of the batch's `ordinal`th scenario.
    fn random(&self, ordinal: usize) -> Random {
        Random::new(
            SEED ^ self.wing as u32
                ^ (self.wing >> 32) as u32
                ^ self.region.rotate_left(16)
                ^ ordinal as u32,
        )
    }
}

/// What the game's training gate reads of a
/// region: the A side's (the viewer and array A) bomber and fighter group
/// arrays and the B side's, as the game filled
/// them. The family loops that empty those arrays run only in the gate's
/// other branches, after it, so a region trains exactly when all four are
/// empty; the base-equipment, transport and scout arrays do not count.
#[derive(Default)]
struct TrainingGate {
    /// Whether the participant arrays are known ([`participant_countries`]).
    participants: bool,
    /// Regions where a wing reached a bomber or fighter array, entering it
    /// or finding it (or its side) full.
    grouped: BTreeSet<u32>,
    /// Regions where a wing that may reach one was omitted undecided.
    undecided: BTreeSet<u32>,
}

impl TrainingGate {
    fn reach(&mut self, region: u32, group: usize) {
        if group < 2 {
            self.grouped.insert(region);
        }
    }

    /// A wing omitted before its grouping was decided; `None` when even its
    /// array is unknown.
    fn undecided(&mut self, region: u32, group: Option<usize>) {
        if group.is_none_or(|group| group < 2) {
            self.undecided.insert(region);
        }
    }
}

/// One training wing's batch: `batches` fighter-patrol scenarios of
/// `formation` aircraft over its pool base's region.
struct TrainingDraw<'a, B> {
    wing: &'a AirWingRow,
    base: B,
    region: u32,
    formation: usize,
    batches: usize,
}

/// The training pass, for each region the gate admits.
///
/// Candidates are the viewing country's pooled wings whose mission has mask
/// bit 11, regardless of other wing state, and whose pool base stands in a
/// province assigned to this region. At most 50 are admitted in pool/wing
/// order, recorded as `AirWings.pool_ordinal`. The pool owner is the recorded
/// `AirWings.owner`; a country override does not move a wing out of its
/// pool. The builder then draws candidates at random with a wall-clock seed.
/// The viewer uses its deterministic presentation seed. A training wing
/// whose pool base does not resolve here still took its place in the game,
/// so a region with 50 or more candidates beside one is counted.
///
/// Each wing converts its own plane count to units by dividing by
/// `TRANSPORT_DIVISION_FACTOR`. These units are stepped into kind 1
/// (triple fighter patrol) or kind 0 (single)
/// scenarios against `MAX_TRAINING_SCENARIOS`. A wing below a single, or
/// whose batch is refused, ends the region's pass.
fn training_draws<'a, B>(
    wings: &'a [AirWingRow],
    viewer: impl Fn(&AirWingRow) -> bool,
    base_region: impl Fn(&AirWingRow) -> Result<Option<(u32, B)>, &'static str>,
    gate: &TrainingGate,
    family: SteppedFamily,
    counts: &mut BTreeMap<(u32, String), usize>,
    diagnostics: &mut Vec<String>,
) -> Vec<TrainingDraw<'a, B>> {
    let mut regions = BTreeMap::<u32, Vec<(&AirWingRow, B)>>::new();
    let mut unresolved = 0;
    for wing in wings
        .iter()
        .filter(|wing| wing.mission_mask & AIR_MISSION_TRAINING != 0 && viewer(wing))
    {
        match base_region(wing) {
            Ok(Some((region, base))) => regions.entry(region).or_default().push((wing, base)),
            Ok(None) => {}
            Err(reason) => {
                unresolved += 1;
                diagnostics.push(format!("training wing {} omitted: {reason}", wing.id));
            }
        }
    }
    let (mut unknown, mut undecided, mut uncertain) = (0, 0, 0);
    let mut draws = Vec::new();
    for (region, mut candidates) in regions {
        if gate.grouped.contains(&region) {
            continue;
        }
        if !gate.participants {
            unknown += candidates.len();
            continue;
        }
        if gate.undecided.contains(&region) {
            undecided += 1;
            continue;
        }
        // The game keeps the first 50 in pool/wing order.
        candidates.sort_by_key(|(wing, _)| wing.pool_ordinal);
        if candidates.len() >= GROUP_CAPACITY && unresolved != 0 {
            uncertain += 1;
        }
        candidates.truncate(GROUP_CAPACITY);
        // Random selection with last-entry replacement.
        let mut rng = Random::new(SEED ^ region.rotate_left(16));
        while !candidates.is_empty() {
            let index = rng.next() as usize % candidates.len();
            let (wing, base) = candidates.swap_remove(index);
            let Some((formation, batches)) = family.batches(family.sizing.units(wing.planes))
            else {
                break;
            };
            let count = counts
                .entry((region, format!("fighter_{formation}_patrol")))
                .or_default();
            if *count >= family.sizing.limit {
                break;
            }
            *count += batches;
            draws.push(TrainingDraw {
                wing,
                base,
                region,
                formation,
                batches,
            });
        }
    }
    if unknown != 0 {
        diagnostics.push(format!(
            "{unknown} training wings omitted: the participant arrays that decide their regions' training gate are unknown"
        ));
    }
    if undecided != 0 {
        diagnostics.push(format!(
            "training omitted in {undecided} regions: a wing that may fill their bomber or fighter arrays is undecided"
        ));
    }
    if uncertain != 0 {
        diagnostics.push(format!(
            "{uncertain} regions list at least {GROUP_CAPACITY} training wings beside {unresolved} whose pool base does not resolve; one of those may take a place among the {GROUP_CAPACITY} that fly"
        ));
    }
    draws
}

impl AircraftPlan {
    /// One scenario of `batch` at `province`: the game anchors it at the
    /// province's integer pixel mean.
    fn place(
        &mut self,
        batch: &Batch<'_>,
        province: u32,
        rng: &mut Random,
        map: &Placement<'_, '_>,
    ) {
        let (start, rig, region) = (batch.start, batch.rig, batch.region);
        let Some([x, z]) = map.geometry.centroid(province) else {
            self.diagnostics.push(format!(
                "region {region} province {province} has no centroid"
            ));
            return;
        };
        let origin = [x, map.heights.height_at_world(x, z), z];
        let direction = sub(origin, start);
        let yaw = -direction[0].atan2(direction[2])
            + (10.0 + 30.0 * rng.unit().sqrt()).to_radians()
                * if rng.next() & 1 == 0 { -1.0 } else { 1.0 };
        let phase = rng.unit();
        let mut group = Vec::new();
        let mut group_travel = 0.0f32;
        for (track, poses) in rig.tracks.iter().enumerate() {
            // The game attempts both sides independently and synchronizes
            // only actors successfully appended to the scenario.
            let Some(actor) = (if rig.sides[track] == 0 {
                batch.attacker
            } else {
                batch.defender
            }) else {
                continue;
            };
            let first = poses[0];
            let last = *poses.last().unwrap();
            let begin = add(origin, rotate(first.translation, yaw));
            let end = add(origin, rotate(last.translation, yaw));
            let outgoing = Route::build(
                actor.start,
                begin,
                rotate(first.basis[2], yaw),
                true,
                map.heights,
                map.defines,
            );
            let incoming = Route::build(
                end,
                actor.start,
                rotate(last.basis[2], yaw),
                false,
                map.heights,
                map.defines,
            );
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
                mesh: actor.mesh,
                wing: actor.wing,
                region,
                rig: batch.key.to_owned(),
                track,
                origin,
                yaw,
                outgoing,
                incoming,
                travel,
                delay: 0.0,
                group_travel: 0.0,
                phase,
                requests: Arc::default(),
            });
        }
        for f in &mut group {
            f.delay = group_travel - f.travel;
            f.group_travel = group_travel;
        }
        self.flights.extend(group);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build(
        frame: &DecodedFrame,
        assets: &AircraftAssets,
        content: &CounterContent,
        geometry: &ProvinceGeometry,
        heights: &HeightField<'_>,
        vision: &Vision,
        model: &dyn Fn(u64, u32) -> Result<Option<String>, String>,
        carrier_origin: &dyn Fn(&TaskForceRow) -> Result<[f32; 3], &'static str>,
    ) -> io::Result<Self> {
        let mut out = Self::default();
        let links: BTreeMap<_, _> = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|r| (r.id, r.canonical_slot))
            .collect();
        // Admission uses the viewing country independently of the fog-of-war switch.
        let ours = vision.ours();
        let (participants, gap) = participant_countries(frame, ours, &links)?;
        let mut training = TrainingGate {
            participants: gap.is_none(),
            ..TrainingGate::default()
        };
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
            tactical: needs(1 << 1)
                .then(|| TacticalTargetSources::read(frame, &wars))
                .transpose()?,
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
        let flight_defines = match &assets.flight_defines {
            Ok(defines) => defines,
            Err(error) => {
                out.diagnostics.push(format!(
                    "flying aircraft omitted: content NAirGfx defines unavailable: {error}"
                ));
                return Ok(out);
            }
        };
        let carriers = CarrierSources::read(frame, &registry, carrier_origin)?;
        let naval_patrol = needs(AIR_MISSION_NAVAL_PATROL)
            .then(|| NavalPatrolSources::read(frame))
            .transpose()?
            .flatten();
        let placement = Placement {
            geometry,
            heights,
            defines: flight_defines,
        };
        let wing_mesh = |wing: u64, raw_country: u32| {
            let variant = super::aircraft_model::wing_primary(frame, wing)
                .map_err(Unresolved::Token)?
                .ok_or(Unresolved::Identity)?;
            let entity = model(variant, raw_country)
                .map_err(Unresolved::Token)?
                .ok_or_else(|| Unresolved::Missing(format!("variant {variant}")))?;
            assets.resolve(&entity)
        };
        let mut refusals = Refusals::default();
        let mut groups = Groups::default();
        let mut caps = Caps::default();
        let mut undecided = Undecided::default();
        let mut regional = BTreeMap::<u32, RegionGroups>::new();
        for visit in visit_order(wings, registrations, &links, ours, &participants)? {
            let (region, wing) = (visit.region, visit.wing);
            if !passes_initial_gate(wing) {
                continue;
            }
            let Some(inputs) = sources.mission(wing.id) else {
                out.diagnostics
                    .push(format!("wing {} lacks its mission operands", wing.id));
                training.undecided(region, None);
                continue;
            };
            // A wing without planes flies only for these definitions.
            if wing.planes == 0 && inputs.definition_map_flags & AIR_MISSION_ZERO_PLANE_FLAGS == 0 {
                continue;
            }
            let Some(category) = air_reachability::category(
                wing.mission_mask,
                inputs.last_target_province,
                inputs.definition_map_flags,
            ) else {
                continue;
            };
            let group = group_index(category);
            let owner = effective_country(wing);
            let Some(&model_slot) = links.get(&owner) else {
                training.undecided(region, Some(group));
                continue;
            };
            // The game gets a scout array only in the viewing country's own
            // call. A transfer is rejected when its base resolves, rather
            // than flying from it.
            if (category == Category::Scout && !visit.own)
                || registry.transfer_resolves(wing.base_override)
            {
                continue;
            }
            let Some(base) = registry.bases.get(&wing.air_base) else {
                out.diagnostics
                    .push(format!("wing {} pool base does not resolve", wing.id));
                training.undecided(region, Some(group));
                continue;
            };
            let Some(mission_country) = mission_countries.resolve(wing) else {
                out.diagnostics.push(format!(
                    "wing {} mission country cannot be derived from this hour's sources",
                    wing.id
                ));
                training.undecided(region, Some(group));
                continue;
            };
            match sources.host_admits(wing, base) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(reason) => {
                    out.diagnostics
                        .push(format!("wing {} omitted: {reason}", wing.id));
                    training.undecided(region, Some(group));
                    continue;
                }
            }
            match scans.admission(wing, mission_country, region, inputs) {
                TargetAdmission::Allowed => {}
                TargetAdmission::Blocked => continue,
                TargetAdmission::Unknown(causes) => {
                    undecided.record(causes);
                    training.undecided(region, Some(group));
                    continue;
                }
            }
            // The wing enters its array, or finds it (or its side) full.
            training.reach(region, group);
            if groups.admit(&visit, group, inputs.last_target_province) {
                regional.entry(region).or_default().arrays[visit.side][group].push(ScenarioWing {
                    wing,
                    inputs,
                    base,
                    mission_country,
                    country: model_slot,
                    side: visit.side,
                });
            }
        }
        let history = ScenarioHistory::read(frame, ours)?;
        let count = |candidate: &ScenarioWing<'_>| {
            sources.count(
                candidate.wing,
                candidate.inputs,
                candidate.base,
                candidate.mission_country.slot,
                (effective_country(candidate.wing), candidate.country),
                defines,
            )
        };
        let naval = |candidate: &ScenarioWing<'_>, rng: &mut Random| {
            let reached = candidate.inputs.reached_provinces.as_slice();
            if reached.is_empty() {
                return Err("no reached province to anchor its scenario");
            }
            let sources = naval_patrol
                .as_ref()
                .ok_or("complete naval task-force combats and histories are required")?;
            let (outcome, province) = sources.choose(
                candidate.country,
                candidate.wing.mission_region.unwrap(),
                ours,
                reached,
                rng,
                &assets.province_kinds,
            );
            Ok((Some(province), outcome))
        };
        for (region, groups) in regional {
            let mut rng = Random::new(SEED ^ region.rotate_left(16));
            let history = history
                .as_ref()
                .map_err(|reason| *reason)
                .and_then(|history| history.region(region));
            let draws = scenarios::generate(
                region,
                groups,
                *flight_defines,
                history,
                &count,
                &mut rng,
                &mut caps,
                &mut out.diagnostics,
                &naval,
            );
            let mut targets = BTreeMap::new();
            for draw in draws {
                let candidate = draw.attacker;
                let wing = candidate.wing;
                let reached = candidate.inputs.reached_provinces.as_slice();
                let outcome = draw.outcome;
                let per_scenario = candidate.side == 1 && matches!(draw.kind, 0 | 1);
                let province =
                    if let Some(province) = targets.get(&draw.batch).filter(|_| !per_scenario) {
                        *province
                    } else if let Some(province) = draw.province {
                        province
                    } else if candidate.side == 1 && matches!(draw.kind, 0 | 1) {
                        let Some(provinces) = assets
                            .region_draws
                            .get(&region)
                            .filter(|list| !list.is_empty())
                        else {
                            out.diagnostics.push(format!(
                                "wing {} has no region province to anchor its scenario",
                                wing.id
                            ));
                            continue;
                        };
                        provinces[rng.next() as usize % provinces.len()]
                    } else if reached.is_empty() {
                        out.diagnostics.push(format!(
                            "wing {} has no reached province to anchor its scenario",
                            wing.id
                        ));
                        continue;
                    } else {
                        reached[rng.next() as usize % reached.len()]
                    };
                if !per_scenario {
                    targets.insert(draw.batch, province);
                }
                // The scenario's orientation reference remains available
                // even when the designated attacking entity is refused.
                let start = match take_off(content, &carriers, candidate.base) {
                    Ok(start) => start,
                    Err(reason) => {
                        out.diagnostics
                            .push(format!("wing {} has no take-off point: {reason}", wing.id));
                        continue;
                    }
                };
                let mut actor = |candidate: ScenarioWing<'_>,
                                 start: Result<[f32; 3], &'static str>|
                 -> Option<Actor> {
                    let start = match start {
                        Ok(start) => start,
                        Err(reason) => {
                            out.diagnostics.push(format!(
                                "wing {} has no take-off point: {reason}",
                                candidate.wing.id
                            ));
                            return None;
                        }
                    };
                    match wing_mesh(candidate.wing.id, effective_country(candidate.wing)) {
                        Ok(mesh) => Some(Actor {
                            mesh,
                            wing: candidate.wing.id,
                            start,
                        }),
                        Err(error) => {
                            refusals.omit(candidate.wing.id, error, &mut out.diagnostics);
                            None
                        }
                    }
                };
                let attacker = actor(candidate, Ok(start));
                let defender = draw.defender.and_then(|candidate| {
                    actor(candidate, take_off(content, &carriers, candidate.base))
                });
                let key = rig_key(scenario_name(draw.kind), outcome);
                let Some(rig) = assets.rigs.get(&key) else {
                    out.diagnostics.push(format!("missing scenario {key}"));
                    continue;
                };
                let batch = Batch {
                    wing: wing.id,
                    region,
                    key: &key,
                    rig,
                    start,
                    attacker,
                    defender,
                };
                out.place(&batch, province, &mut rng, &placement);
            }
        }
        let draws = training_draws(
            wings,
            |wing| links.get(&wing.owner) == Some(&ours),
            |wing| {
                // Resolve the wing pool's base and province; the game dereferences
                // both results without a null check.
                let base = registry
                    .bases
                    .get(&wing.air_base)
                    .ok_or("its pool base does not resolve")?;
                let province = match base.state {
                    Some(_) => base.province.ok_or("its state base has no province")?,
                    None => carriers.province(base)?,
                };
                // A province omitted from every region has no assigned region.
                Ok(content.region(province).map(|region| (region, base)))
            },
            &training,
            flight_defines.training,
            &mut caps.counts,
            &mut out.diagnostics,
        );
        for draw in draws {
            let (wing, region) = (draw.wing, draw.region);
            let Some(_) = links.get(&effective_country(wing)) else {
                out.diagnostics.push(format!(
                    "training wing {} omitted: its effective country is not linked",
                    wing.id
                ));
                continue;
            };
            let start = match take_off(content, &carriers, draw.base) {
                Ok(start) => start,
                Err(reason) => {
                    out.diagnostics
                        .push(format!("wing {} has no take-off point: {reason}", wing.id));
                    continue;
                }
            };
            let mesh = match wing_mesh(wing.id, effective_country(wing)) {
                Ok(mesh) => mesh,
                Err(error) => {
                    refusals.omit(wing.id, error, &mut out.diagnostics);
                    continue;
                }
            };
            let key = format!("fighter_{}_patrol", draw.formation);
            let Some(rig) = assets.rigs.get(&key) else {
                out.diagnostics.push(format!("missing scenario {key}"));
                continue;
            };
            // Draw one random province from the region's list for every scenario of this wing.
            let Some(provinces) = assets
                .region_draws
                .get(&region)
                .filter(|provinces| !provinces.is_empty())
            else {
                out.diagnostics.push(format!(
                    "training wing {} omitted: region {region} has no province",
                    wing.id
                ));
                continue;
            };
            let province =
                provinces[wing_random(wing.id, region).next() as usize % provinces.len()];
            let batch = Batch {
                wing: wing.id,
                region,
                key: &key,
                rig,
                start,
                attacker: Some(Actor {
                    mesh,
                    wing: wing.id,
                    start,
                }),
                defender: None,
            };
            for ordinal in 0..draw.batches {
                out.place(&batch, province, &mut batch.random(ordinal), &placement);
            }
        }
        for flight in &mut out.flights {
            flight.requests = Arc::new(flight_requests(
                flight,
                &assets.rigs[&flight.rig],
                assets.library().rig(flight.mesh),
                flight_defines.clock_speed,
            ));
        }
        out.diagnostics.extend(undecided.diagnostics());
        out.diagnostics.extend(refusals.diagnostics());
        Ok(out)
    }

    fn placed(&self, assets: &AircraftAssets, seconds: f32) -> Vec<(usize, ModelInstance)> {
        let Ok(defines) = &assets.flight_defines else {
            return Vec::new();
        };
        self.flights
            .iter()
            .enumerate()
            .filter_map(|(index, flight)| {
                let rig = &assets.rigs[&flight.rig];
                let total = 2.0 * flight.group_travel + rig.duration;
                let elapsed = seconds.max(0.0) * defines.clock_speed + flight.phase * total;
                let time = elapsed.rem_euclid(total);
                let (pose, scale) = if time < flight.group_travel {
                    if time < flight.delay {
                        return None;
                    }
                    let t = (time - flight.delay) / flight.travel;
                    (flight.outgoing.at(t), defines.takeoff_scale(t))
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
                    (flight.incoming.at(t), defines.landing_scale(t))
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
                        // Preserve the rig/route basis during matrix upload; Euler conversion
                        // above is only the retained port's instance declaration.
                        native_flags: 4,
                        native_basis: pose.basis,
                        train_clip: [0.0; 4],
                        animation_seed: index as u32,
                        animation_epoch: (elapsed / total).floor() as u32,
                        animation_phase: (time - flight.delay) / defines.clock_speed,
                        ..ModelInstance::default()
                    },
                ))
            })
            .collect()
    }

    pub fn field(&self, assets: &AircraftAssets, seconds: f32) -> ModelField {
        let placed = self.placed(assets, seconds);
        let mut field = ModelField::build(
            &assets.library(),
            placed
                .iter()
                .map(|(index, instance)| (self.flights[*index].mesh, *instance))
                .collect(),
            128.0,
        );
        for (index, instance) in placed {
            let flight = &self.flights[index];
            field.clocks.insert(
                (
                    flight.mesh,
                    instance.animation_seed,
                    instance.animation_epoch,
                ),
                Clock {
                    time: instance.animation_phase,
                    phase: None,
                    requests: Arc::clone(&flight.requests),
                },
            );
        }
        field
    }

    pub fn report(&self, assets: &AircraftAssets) -> Vec<String> {
        let mut lines = vec![format!(
            "Aircraft: {} generated flight slots, {} at time zero, seed 0x{SEED:08x}; live presentation phase is unrecorded",
            self.flights.len(),
            self.placed(assets, 0.0).len()
        )];
        let mut wings: BTreeMap<(u64, u32, &str, usize), usize> = BTreeMap::new();
        for f in &self.flights {
            *wings.entry((f.wing, f.region, &f.rig, f.mesh)).or_default() += 1;
        }
        for ((wing, region, rig, mesh), count) in wings {
            let library = assets.library();
            lines.push(format!(
                "Aircraft wing {wing}, region {region}: {count} slots, {rig}, {}, asset scale {}",
                library.meshes[mesh].name, library.meshes[mesh].scale
            ));
        }
        for (index, i) in self.placed(assets, 0.0) {
            let flight = &self.flights[index];
            lines.push(format!(
                "Aircraft t=0 wing {} track {} {} at {:.4},{:.4},{:.4} instance scale {:.4}",
                flight.wing,
                flight.track,
                assets.library().meshes[flight.mesh].name,
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
        lines.extend(
            assets
                .lazy_diagnostics()
                .into_iter()
                .map(|(_, reason)| reason),
        );
        lines
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "aircraft/carrier_tests.rs"]
mod carrier_tests;
