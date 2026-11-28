use content_reader_mesh::mesh::MeshProducts as _;

#[cfg(test)]
use viewer_models::aircraft_events::callback;
use viewer_models::aircraft_events::scenario_side_requests;
use dashboard_core::protocol::AirMission;
use std::sync::Mutex;
use std::sync::RwLock;
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    ops::Range,
    sync::Arc,
    time::Instant,
};
use viewer_unit_reads::aircraft_model;
use viewer_gpu::camera::Camera;
use dashboard_text::table_access::{TokenKeys, TokenNames};

use engine_air_history::air_kamikaze::window_sum;
mod launches;
mod model_inputs;
mod scenarios;
use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_map::map::{MapData, ProvinceKind};
use content_reader_mesh::mesh::{self, Transform};
use content_reader_equipment_graphics::equipment_graphics::aircraft as aircraft_catalogue;
use content_reader_image::native::Decode;
use content_reader_regions::regions::StrategicRegion;
pub use model_inputs::{KeptModels, ModelMemo, ModelsMade, Selected};
use replay_data::{
    rows::AIR_MISSION_LAY_MINES, rows::AIR_MISSION_NAVAL_PATROL, rows::AIR_MISSION_TRAINING,
    rows::AIR_MISSION_ZERO_PLANE_FLAGS, rows::AirBaseRow, rows::AirCountryRelationRow,
    rows::AirMissionInputRow, rows::AirWingRow, rows::ShipRow, rows::TaskForceRow, rows::tables,
};
use replay_frame::frame::{DecodedFrame, ViewIdentity};
use scenarios::{RegionGroups, ScenarioWing};

use viewer_models::{
    entity_animation::{Clock, Request},
    ModelField,
    ModelInstance,
    ModelLibrary,
    PaletteJoint,
};
use viewer_unit_reads::{
    air_participants::{AirParticipants, CountryRelations, CountryVectors, ordered_participants},
    air_reachability::{self, Category, Causes, MineTargetSources, MissionCountry, NavalCombatTargetSources, NavalOccupantSources, PortTargetSources, RegionFractions, RegionStates, ScoutTargetSources, StrategicTargetSources, TacticalTargetSources, TargetAdmission, Wars},
    air_registry_sources::AirRegistrySources,
};
use replay_derive::aircraft_count::{SortieDefines, SortieSources};
use viewer_gpu::{province_geometry::ProvinceGeometry, province_map::HeightField};
use viewer_foundation::{tables::{Available, core, rows}, visibility::Families};

pub const SEED: u32 = 0x4149_5250;

#[derive(Clone, Copy, Debug, PartialEq)]
struct FamilyDefines {
    factor: i32,
    triple: i32,
    limit: usize,
}

impl FamilyDefines {
    fn load(defines: &Defines, factor: &str, anim: &str, limit: &str) -> io::Result<Self> {
        let factor_value = defines.int32("NAirGfx", factor)?;
        if factor_value == 0 {
            return Err(invalid_data(format!("NDefines.NAirGfx.{factor} is zero")));
        }
        Ok(Self {
            factor: factor_value,
            triple: defines.int32("NAirGfx", &format!("AIRPLANES_3_{anim}_ANIM"))?,
            limit: defines.int32("NAirGfx", limit)?.max(1) as usize,
        })
    }

    fn units(self, planes: i32) -> i32 {
        if planes > 0 {
            (planes / self.factor).max(1)
        } else {
            0
        }
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlightDefines {
    fighter: SteppedFamily,
    scout: SteppedFamily,
    bomber: FamilyDefines,
    bomber_single: i32,
    dogfight: SteppedFamily,
    fire_threshold: i64,
    carpet_threshold: i64,
    transport: SteppedFamily,
    training: SteppedFamily,
    missile: Result<FamilyDefines, &'static str>,
    clock_speed: f32,
    curve_density: f32,
    curve_extrapolation: f32,
    curve_elevation: f32,
    scale_takeoff: f32,
    scale_landing: f32,
    scale_min: f32,
    smooth_move: f32,
    smooth_turn: f32,
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
            missile: match (
                defines.int32("NAirGfx", "MISSILES_DIVISION_FACTOR"),
                defines.int32("NAirGfx", "MAX_MISSILE_BOMBING_SCENARIOS"),
            ) {
                (Err(_), _) => Err("NDefines.NAirGfx.MISSILES_DIVISION_FACTOR is unreadable"),
                (Ok(0), _) => Err("NDefines.NAirGfx.MISSILES_DIVISION_FACTOR is zero"),
                (_, Err(_)) => Err("NDefines.NAirGfx.MAX_MISSILE_BOMBING_SCENARIOS is unreadable"),
                (Ok(factor), Ok(limit)) => Ok(FamilyDefines {
                    factor,
                    triple: 0,
                    limit: limit.max(1) as usize,
                }),
            },
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
            smooth_move: float("AIRPLANES_SMOOTH_INTERPOLATION_MOVE")?,
            smooth_turn: float("AIRPLANES_SMOOTH_INTERPOLATION_TURN")?,
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
        if out.curve_density.is_nan() || out.curve_density <= 0.0 {
            return Err(invalid_data(
                "NDefines.NAirGfx.AIRPLANES_CURVE_POINT_DENSITY must be positive".into(),
            ));
        }
        Ok(out)
    }

    #[cfg(test)]
    fn fixture() -> Self {
        let sizing = FamilyDefines {
            factor: 1,
            triple: 4,
            limit: 5,
        };
        let stepped = SteppedFamily { sizing, single: 1 };
        Self {
            fighter: stepped,
            scout: stepped,
            bomber: sizing,
            bomber_single: 1,
            dogfight: stepped,
            transport: stepped,
            training: stepped,
            missile: Ok(sizing),
            fire_threshold: 370_000,
            carpet_threshold: 240_000,
            clock_speed: 1.0,
            curve_density: 2.5,
            curve_extrapolation: 17.0,
            curve_elevation: 5.5,
            scale_takeoff: 0.2,
            scale_landing: 0.8,
            scale_min: 0.15,
            smooth_move: 0.25,
            smooth_turn: 0.18,
        }
    }

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundContact {
    pub offset: f32,
    pub explosion: f32,
}

impl GroundContact {
    pub fn load(defines: &Defines) -> Result<Self, &'static str> {
        Band::load(defines).map(Self::from)
    }

    pub fn exploded(&self, played: Option<Option<(f32, f32)>>) -> bool {
        match played {
            Some(Some((position, _))) => position >= self.explosion,
            Some(None) => 0.0 >= self.explosion,
            None => false,
        }
    }
}

impl From<Band> for GroundContact {
    fn from(band: Band) -> Self {
        Self {
            offset: band.offset,
            explosion: band.explosion_delay,
        }
    }
}

fn threshold_raw(value: f32) -> i64 {
    let rounded = value * 100_000.0 + (if value >= 0.0 { 1.0 } else { 0.0 }) - 0.5;
    let limit = -(i64::MIN as f32);
    if !(-limit..limit).contains(&rounded) {
        i64::MIN
    } else {
        rounded as i64
    }
}

pub fn draws(camera: &Camera) -> bool {
    camera.height < camera.defines.map_objects_cutoff
}

#[derive(Debug, Clone, Default)]
struct Equipment {
    archetype: String,
    sprite: String,
}

#[derive(Debug)]
struct Rig {
    tracks: Vec<Vec<Transform>>,
    sides: Vec<u8>,
    fps: f32,
    duration: f32,
    events: Vec<mesh::EntityEvent>,
    speed: f32,
    unbound: Vec<Option<f32>>,
    lands: Vec<bool>,
    rockets: Vec<bool>,
}

const UNBOUND_SPEED: f32 = 1.0;

const FAIL: u8 = 3;

fn lands(kind: u8, outcome: u8, side: u8) -> bool {
    !match kind {
        scenarios::MISSILE_KIND => outcome == 0,
        16 | 17 => outcome == SUCCESS,
        12 | 18 => side == 1 && outcome == SUCCESS,
        13 | 14 => (side == 0 && outcome == FAIL) || (side == 1 && outcome == SUCCESS),
        _ => false,
    }
}

const EXPLODE_STATE: &str = "explode";

#[derive(Clone, Copy, Debug, PartialEq)]
struct Band {
    offset: f32,
    explosion_delay: f32,
}

impl Band {
    fn load(defines: &Defines) -> Result<Self, &'static str> {
        Ok(Self {
            offset: defines
                .number("NAirGfx", "AIRPLANES_GROUND_COLLISION_OFFSET_Y")
                .map_err(|_| "NDefines.NAirGfx.AIRPLANES_GROUND_COLLISION_OFFSET_Y is unreadable")?
                as f32,
            explosion_delay: defines
                .number("NAirGfx", "AIRPLANES_GROUND_EXPLOSION_TIME_DELAY")
                .map_err(
                    |_| "NDefines.NAirGfx.AIRPLANES_GROUND_EXPLOSION_TIME_DELAY is unreadable",
                )? as f32,
        })
    }
}

#[derive(Debug)]
pub struct AircraftAssets {
    library: RwLock<ModelLibrary>,
    roster: Arc<mesh::Roster>,
    layers: ResolvedLayers,
    unusable: Mutex<BTreeMap<String, String>>,
    bakes: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
    #[cfg(test)]
    resolve_hooks: tests::ResolveHooks,
    lazy_diagnostics: Mutex<Vec<content_reader_parser::Diagnostic>>,
    keys: TokenKeys,
    rigs: BTreeMap<String, Rig>,
    region_provinces: BTreeMap<u32, Vec<u32>>,
    region_draws: BTreeMap<u32, Vec<u32>>,
    region_fractions: BTreeMap<u32, RegionFractions>,
    province_kinds: BTreeMap<u32, ProvinceKind>,
    sortie_defines: Result<SortieDefines, String>,
    flight_defines: Result<FlightDefines, String>,
    band: Option<Band>,
    launch_content: launches::Content,
    ground: Result<GroundContact, String>,
}

#[cfg(test)]
pub(crate) fn candidates(tag: &str, name: &str, archetype: &str, sprite: &str) -> Vec<String> {
    aircraft_model::candidates("", tag, name, archetype, sprite)
}

const NOT_LOADED: &str = "its mesh did not load";

#[derive(Debug, PartialEq)]
enum Unresolved {
    Identity,
    Missing(String),
    Token(String),
    Refused { entity: String, reason: String },
}

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

fn plane_library<'a>(
    layers: &ResolvedLayers,
    decode: Decode,
    roster: &mesh::Roster,
    declared: impl IntoIterator<Item = &'a String>,
    warnings: &mut Vec<content_reader_parser::Diagnostic>,
) -> io::Result<(ModelLibrary, BTreeMap<String, String>)> {
    let mut unusable = BTreeMap::new();
    let mut names = Vec::new();
    for entity in declared {
        match roster.refused_declaration(entity) {
            Some(reason) => {
                unusable.insert(entity.clone(), reason);
            }
            None => names.push(entity.clone()),
        }
    }
    if !unusable.is_empty() {
        warnings.push((content_reader_parser::DiagnosticKind::Refused, format!(
            "Aircraft entities are refused, and wings whose aircraft resolves to one are not drawn: {}",
            unusable
                .iter()
                .map(|(entity, reason)| format!("{entity} ({reason})"))
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    let mut library = ModelLibrary::load_with(
        layers,
        decode,
        &roster.descriptors,
        &names,
        warnings,
        mesh::Roster::refused_declaration,
    )?;
    let recorded = layers.recorded_descriptor_roster()?;
    let from_recording: Vec<String> = unusable
        .keys()
        .filter(|entity| {
            recorded.as_ref().is_some_and(|recorded| {
                recorded.descriptors.entities.contains_key(*entity)
                    && recorded.refused_declaration(entity).is_none()
            })
        })
        .cloned()
        .collect();
    if !from_recording.is_empty() {
        let baked = ModelLibrary::load_recorded(layers, decode, &from_recording, |entity| {
            unusable.get(entity).cloned().unwrap_or_default()
        })?;
        library.merge(baked);
        for entity in &from_recording {
            if library.slot(entity).is_some() {
                unusable.remove(entity);
            }
        }
    }
    for name in names {
        if library.slot(&name).is_none() {
            let reason = match library.failed.get(&name) {
                Some(why) => format!("{NOT_LOADED}: {why}"),
                None => NOT_LOADED.to_owned(),
            };
            unusable.insert(name, reason);
        }
    }
    for entity in unusable.keys() {
        layers.note_refused(format!("model entity {entity}"));
    }
    Ok((library, unusable))
}

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

fn scenario<'a>(
    descriptors: &'a mesh::Descriptors,
    entity: &str,
    state: &str,
) -> Result<(&'a mesh::MeshDefinition, &'a mesh::AnimationDefinition, f32), String> {
    let inherited = descriptors
        .inherit(entity)
        .ok_or("the content declares no such entity")?;
    let mesh = inherited.mesh.ok_or("it names no pdxmesh")?;
    let definition = descriptors
        .meshes
        .get(mesh)
        .ok_or_else(|| format!("its pdxmesh {mesh} is not declared"))?;
    let state = inherited
        .states
        .get(state)
        .and_then(|states| states.first())
        .ok_or_else(|| format!("it declares no {state} state"))?;
    let (_, name) = definition
        .animations
        .iter()
        .find(|(declared, _)| *declared == state.animation)
        .ok_or_else(|| {
            format!(
                "its pdxmesh {mesh} declares no {} animation",
                state.animation
            )
        })?;
    let animation = descriptors
        .animations
        .get(name)
        .ok_or_else(|| format!("its animation {name} is not declared"))?;
    Ok((definition, animation, state.speed))
}

fn scenario_source(
    layers: &ResolvedLayers,
    descriptors: &mesh::Descriptors,
    entity: &str,
    definition: &mesh::MeshDefinition,
) -> Result<viewer_models::mesh_pose::Source, String> {
    let unreadable =
        |error: io::Error| format!("its mesh file {} does not read: {error}", definition.file);
    let tree = mesh::parse_in(layers, &definition.file).map_err(unreadable)?;
    let skeleton = mesh::native_skeleton(&tree, definition.scale).map_err(unreadable)?;
    Ok(viewer_models::mesh_pose::Source {
        pdxmesh: true,
        binary_locators: viewer_models::mesh_pose::binary_locators(&tree, &skeleton, definition.scale)
            .map_err(unreadable)?,
        entity_locators: viewer_models::mesh_pose::entity_locators(
            &descriptors.entities[entity].locators,
            &skeleton,
            Some(&definition.name),
        ),
        skeleton,
        mesh_scale: definition.scale,
        entity_scale: descriptors
            .inherit(entity)
            .and_then(|inherited| inherited.scale)
            .unwrap_or(1.0),
        frozen: None,
        game_data: None,
    })
}

fn scenario_refusal(roster: &mesh::Roster, entity: &str, state: &str) -> Option<String> {
    if let Some(flaw) = roster.unreadable(entity) {
        return Some(format!("it {flaw}"));
    }
    let descriptors = &roster.descriptors;
    let inherited = descriptors.inherit(entity)?;
    let mesh = inherited.mesh?;
    if let Some(flaw) = roster.unreadable(mesh) {
        return Some(format!("its mesh {mesh} {flaw}"));
    }
    let state = inherited.states.get(state)?.first()?;
    let (_, animation) = descriptors
        .meshes
        .get(mesh)?
        .animations
        .iter()
        .find(|(declared, _)| *declared == state.animation)?;
    roster
        .unreadable(animation)
        .map(|flaw| format!("its animation {animation} {flaw}"))
}

const SUCCESS: u8 = 2;

fn state_name(outcome: u8) -> &'static str {
    match outcome {
        1 => "pass",
        2 => "success",
        3 => "fail",
        _ => "default",
    }
}

fn scenario_state<'a>(descriptors: &'a mesh::Descriptors, entity: &str, outcome: u8) -> &'a str {
    let name = state_name(outcome);
    match descriptors.inherit(entity) {
        Some(inherited) if outcome == 0 && !inherited.states.contains_key(name) => {
            inherited.default_state.unwrap_or(mesh::DEFAULT_STATE)
        }
        _ => name,
    }
}

fn rig_key(scenario: &str, outcome: u8) -> String {
    if outcome != 0 {
        format!("{scenario}_{}", state_name(outcome))
    } else {
        scenario.to_owned()
    }
}

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
        15 => "missile_1_bombing",
        16 => "fighter_1_kamikaze",
        17 => "fighter_3_kamikaze",
        18 => "fighter_1_vs_transport_1",
        19 => "transport_1_supplying",
        20 => "transport_3_supplying",
        _ => unreachable!("conventional aircraft scenario kind"),
    }
}

fn scenario_mission(kind: u8) -> AirMission {
    match kind {
        0 | 1 => AirMission::Patrol,
        2 | 3 => AirMission::Reconnaissance,
        4 | 5 => AirMission::Support,
        6..=11 => AirMission::Bombing,
        scenarios::MISSILE_KIND => AirMission::Missile,
        16 | 17 => AirMission::Kamikaze,
        19 | 20 => AirMission::Supply,
        _ => AirMission::Combat,
    }
}

fn place_flights(
    flights: &[Flight],
    rigs: &BTreeMap<String, Rig>,
    defines: &FlightDefines,
    seconds: f32,
) -> Vec<(usize, ModelInstance)> {
    place_runs(flights, rigs, defines, seconds, &|_| None)
}

fn place_runs(
    flights: &[Flight],
    rigs: &BTreeMap<String, Rig>,
    defines: &FlightDefines,
    seconds: f32,
    run: &dyn Fn(usize) -> Option<(f32, u32, Ground)>,
) -> Vec<(usize, ModelInstance)> {
    flights
        .iter()
        .enumerate()
        .filter_map(|(index, flight)| {
            let rig = &rigs[&flight.rig];
            let total = flight.cycle(rig.duration);
            if total.is_nan() || total <= 0.0 {
                return None;
            }
            let elapsed = seconds.max(0.0) * defines.clock_speed + flight.phase * total;
            let (time, cycle, ground) = match run(index) {
                Some((start, cycle, ground)) => (elapsed - start, cycle, ground),
                None => (
                    elapsed.rem_euclid(total),
                    (elapsed / total).floor() as u32,
                    Ground::Following,
                ),
            };
            let (pose, scale) = if time < flight.group_travel {
                if time < flight.delay {
                    return None;
                }
                let t = (time - flight.delay) / flight.travel;
                (flight.outgoing.at(t), defines.takeoff_scale(t))
            } else if time < flight.group_travel + rig.duration {
                let rig_time = time - flight.group_travel;
                match ground {
                    Ground::Exploding { pose, .. } => (pose, 1.0),
                    Ground::Following | Ground::Hidden => {
                        let pose = rig_pose(&rig.tracks[flight.track], rig.fps, rig_time);
                        (
                            Transform {
                                translation: add(
                                    flight.origin,
                                    rotate(pose.translation, flight.yaw),
                                ),
                                basis: pose.basis.map(|axis| rotate(axis, flight.yaw)),
                            },
                            1.0,
                        )
                    }
                }
            } else {
                let t = (time - flight.group_travel - rig.duration) / flight.travel;
                if t >= 1.0 || !flight.lands {
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
                    native_flags: 4,
                    native_basis: pose.basis,
                    train_clip: [0.0; 4],
                    animation_seed: index as u32,
                    animation_epoch: cycle,
                    animation_phase: (time - flight.delay) / defines.clock_speed,
                    ..ModelInstance::default()
                },
            ))
        })
        .collect()
}

fn hidden_flights(
    flights: &[Flight],
    placed: &[(usize, ModelInstance)],
    shown: &dyn Fn(AirMission) -> bool,
) -> BTreeSet<(usize, u32, u32)> {
    placed
        .iter()
        .filter(|(index, _)| !shown(flights[*index].mission))
        .map(|(index, instance)| {
            let (seed, identity) = instance.animation_key();
            (flights[*index].mesh, seed, identity)
        })
        .collect()
}

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
        15 => single("rocket"),
        16 => single("fighter_1"),
        _ => unreachable!("conventional aircraft scenario kind"),
    }
}

fn scenario_tracks(
    source: &viewer_models::mesh_pose::Source,
    animation: &mesh::Animation,
    bones: &[(u8, String)],
) -> io::Result<(Vec<Vec<Transform>>, Vec<bool>)> {
    use viewer_models::mesh_pose::{self, LookupMode};
    let bindings: Vec<_> = bones
        .iter()
        .map(|(_, bone)| mesh_pose::lookup(source, bone, LookupMode::Naval))
        .collect();
    let transform = |matrix: mesh_pose::Matrix| Transform {
        translation: [matrix[3][0], matrix[3][1], matrix[3][2]],
        basis: std::array::from_fn(|column| {
            [matrix[column][0], matrix[column][1], matrix[column][2]]
        }),
    };
    let root = transform(mesh_pose::scale(source.entity_scale));
    let mut tracks = vec![Vec::with_capacity(animation.samples as usize); bindings.len()];
    for frame in 0..animation.samples {
        let pose = mesh_pose::assemble(
            &source.skeleton,
            &animation.frame_trs(frame),
            source.mesh_scale,
            false,
        )?;
        for (track, binding) in tracks.iter_mut().zip(&bindings) {
            let Some(binding) = binding else {
                track.push(root);
                continue;
            };
            let local = binding
                .local(Ok(Some(&pose.joints)))
                .map_err(invalid_data)?;
            track.push(transform(mesh_pose::multiply(
                mesh_pose::scale(source.entity_scale),
                local,
            )));
        }
    }
    Ok((tracks, bindings.iter().map(Option::is_none).collect()))
}

fn scenario_flown(
    layers: &ResolvedLayers,
    roster: &mesh::Roster,
    entity: &str,
    (kind, outcome): (u8, u8),
    bones: &[(u8, String)],
    defines: (&Result<f32, &'static str>, &Result<Band, &'static str>),
    warnings: &mut Vec<content_reader_parser::Diagnostic>,
) -> Option<Rig> {
    let mut refuse = |reason: String| {
        warnings.push((
            content_reader_parser::DiagnosticKind::Refused,
            format!("Aircraft scenario {entity} is refused and not flown: {reason}"),
        ));
    };
    let descriptors = &roster.descriptors;
    let state = scenario_state(descriptors, entity, outcome);
    if let Some(reason) = scenario_refusal(roster, entity, state) {
        refuse(reason);
        return None;
    }
    let (definition, animation, speed) = match scenario(descriptors, entity, state) {
        Ok(scenario) => scenario,
        Err(reason) => {
            refuse(reason);
            return None;
        }
    };
    let loaded = scenario_source(layers, descriptors, entity, definition).and_then(|source| {
        let anim = mesh::read_animation_in(layers, &animation.file)
            .map_err(|error| {
                format!(
                    "its animation file {} does not read: {error}",
                    animation.file
                )
            })?;
        Ok((source, anim))
    });
    let (source, anim) = match loaded {
        Ok(loaded) => loaded,
        Err(reason) => {
            refuse(reason);
            return None;
        }
    };
    let events = descriptors.inherit(entity).unwrap().states[state][0]
        .events
        .clone();
    scenario_rig(
        entity,
        (kind, outcome),
        bones,
        (&source, &anim),
        speed,
        events,
        defines,
        warnings,
    )
}

fn shown_or_recorded<T>(
    layers: &ResolvedLayers,
    entity: &str,
    below: bool,
    mut fly: impl FnMut(bool, &mut Vec<content_reader_parser::Diagnostic>) -> Option<T>,
    warnings: &mut Vec<content_reader_parser::Diagnostic>,
) -> Option<T> {
    let mut notes = Vec::new();
    if let Some(flown) = fly(false, &mut notes) {
        warnings.extend(notes);
        return Some(flown);
    }
    if below {
        let mut recorded = Vec::new();
        if let Some(flown) = fly(true, &mut recorded) {
            let why = notes
                .iter()
                .rev()
                .find(|(kind, _)| *kind == content_reader_parser::DiagnosticKind::Refused)
                .map_or("not flown", |(_, why)| why.as_str());
            layers.note_overlay_refusal(format!("aircraft scenario {entity} ({why})"));
            warnings.extend(recorded);
            return Some(flown);
        }
    }
    layers.note_refused(format!("aircraft scenario {entity}"));
    warnings.extend(notes);
    None
}

#[expect(clippy::too_many_arguments, reason = "the entity, its bones, its animation source and its rocket defines build one rig together")]
fn scenario_rig(
    entity: &str,
    (kind, outcome): (u8, u8),
    bones: &[(u8, String)],
    (source, anim): (&viewer_models::mesh_pose::Source, &mesh::Animation),
    speed: f32,
    events: Vec<mesh::EntityEvent>,
    (rocket_speed, band): (&Result<f32, &'static str>, &Result<Band, &'static str>),
    warnings: &mut Vec<content_reader_parser::Diagnostic>,
) -> Option<Rig> {
    let mut refuse = |reason: String| {
        warnings.push((
            content_reader_parser::DiagnosticKind::Refused,
            format!("Aircraft scenario {entity} is refused and not flown: {reason}"),
        ));
        None
    };
    let (tracks, absent) = match scenario_tracks(source, anim, bones) {
        Ok(tracks) if anim.samples >= 2 => tracks,
        Ok(_) => return refuse("it has fewer than two animation samples".into()),
        Err(reason) => return refuse(format!("its attachment tracks fail: {reason}")),
    };
    if !(speed.is_finite() && speed > 0.0) {
        return refuse("its animation_speed is not positive".into());
    }
    let rocket = kind == scenarios::MISSILE_KIND;
    let landing: Vec<bool> = bones
        .iter()
        .map(|(side, _)| lands(kind, outcome, *side))
        .collect();
    if let (Err(reason), true) = (band, landing.contains(&false)) {
        return refuse(format!(
            "an actor that does not land meets the near-ground band, whose {reason}"
        ));
    }
    let default = if !rocket {
        UNBOUND_SPEED
    } else if !absent.contains(&true) {
        0.0
    } else {
        match rocket_speed {
            Ok(speed) => *speed,
            Err(reason) => {
                return refuse(format!(
                    "its rocket rides the entity root, whose speed {reason}"
                ));
            }
        }
    };
    let mut unbound = Vec::with_capacity(absent.len());
    for ((_, bone), absent) in bones.iter().zip(absent) {
        if absent {
            warnings.push((
                content_reader_parser::DiagnosticKind::Logged,
                format!(
                    "Aircraft scenario {entity} has no attachment node {bone}: the game logs it and that actor rides the scenario entity's root"
                ),
            ));
        }
        unbound.push(absent.then_some(default));
    }
    let fps = anim.fps * speed;
    Some(Rig {
        tracks,
        sides: bones.iter().map(|(side, _)| *side).collect(),
        fps,
        duration: (anim.samples - 1) as f32 / fps,
        events,
        speed,
        unbound,
        lands: landing,
        rockets: vec![rocket; bones.len()],
    })
}

impl AircraftAssets {
    pub fn launch_lifetime(&self, mesh: usize, seed: u32) -> f32 {
        played_end(
            &self.library().rig_source(mesh),
            ModelInstance {
                animation_seed: seed,
                ..launches::Launch::instance_for(0)
            },
        )
    }

    pub fn lazy_diagnostics(&self) -> Vec<content_reader_parser::Diagnostic> {
        self.lazy_diagnostics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn refusals(&self) -> Vec<String> {
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
                .filter(|(kind, _)| *kind == content_reader_parser::DiagnosticKind::Refused)
                .map(|(_, reason)| reason),
        );
        refusals
    }

    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        map: &MapData,
        (defines, graphics): (&Defines, &Defines),
        regions: &BTreeMap<u32, StrategicRegion>,
        warnings: &mut Vec<content_reader_parser::Diagnostic>,
    ) -> io::Result<Self> {
        let database = content_reader_key_names::equipment::Database::shared_complete(layers)?;
        let text = |bytes: &[u8], what: &str| {
            content_reader_parser::parser::text(bytes)
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
        let mut rigs = BTreeMap::new();
        let mut scenarios = Vec::new();
        for kind in 0..=20 {
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
                    kind,
                ));
            }
        }
        let rocket_speed =
            layers.graphics_define("aircraft rockets", graphics, defines, |defines| {
                defines
                    .number("NAirGfx", "ROCKET_SPEED")
                    .map(|speed| speed as f32)
                    .map_err(|_| "NDefines.NAirGfx.ROCKET_SPEED is unreadable")
            });
        let band = layers.graphics_define("aircraft bands", graphics, defines, Band::load);
        let below = layers.without_overlays();
        let below_roster = layers.recorded_descriptor_roster()?;
        let recorded = below.as_ref().zip(below_roster.as_deref());
        for (key, bones, outcome, kind) in scenarios {
            let entity = format!("{key}_entity");
            let flown = shown_or_recorded(
                layers,
                &entity,
                recorded.is_some(),
                |from_recording, notes| {
                    let (stack, roster) = match recorded {
                        Some(recorded) if from_recording => recorded,
                        _ => (layers, &*roster),
                    };
                    scenario_flown(
                        stack,
                        roster,
                        &entity,
                        (kind, outcome),
                        &bones,
                        (&rocket_speed, &band),
                        notes,
                    )
                },
                warnings,
            );
            if let Some(rig) = flown {
                rigs.insert(rig_key(&key, outcome), rig);
            }
        }
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
            library: RwLock::new(library),
            roster: Arc::clone(&roster),
            layers: layers.clone(),
            unusable: Mutex::new(unusable),
            bakes: Default::default(),
            #[cfg(test)]
            resolve_hooks: Default::default(),
            lazy_diagnostics: Default::default(),
            keys: TokenKeys::new(equipment.keys()),
            rigs,
            region_provinces,
            region_draws,
            region_fractions,
            province_kinds,
            sortie_defines: viewer_unit_reads::air_sortie_defines::load_defines(defines)
                .map_err(|error| error.to_string()),
            flight_defines: layers
                .graphics_define("aircraft flights", graphics, defines, FlightDefines::load)
                .map_err(|error| error.to_string()),
            band: band.ok(),
            launch_content: launches::Content::load(
                layers,
                map,
                (defines, graphics),
                &roster,
                warnings,
            ),
            ground: band.map(GroundContact::from).map_err(str::to_owned),
        })
    }

    pub fn ground_contact(&self) -> Result<GroundContact, &str> {
        self.ground.as_ref().copied().map_err(String::as_str)
    }

    pub fn library(&self) -> std::sync::RwLockReadGuard<'_, ModelLibrary> {
        self.library
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn resolve(&self, entity: &str) -> Result<usize, Unresolved> {
        if let Some(known) = self.known(entity) {
            return known;
        }
        #[cfg(test)]
        self.resolve_hooks.reach(entity, tests::ResolvePoint::Cold);
        let claim = self.claim(entity);
        #[cfg(test)]
        {
            self.resolve_hooks
                .reach(entity, tests::ResolvePoint::Claimed);
            if matches!(claim.try_lock(), Err(std::sync::TryLockError::WouldBlock)) {
                self.resolve_hooks
                    .reach(entity, tests::ResolvePoint::Waiting);
            }
        }
        let _bake = claim
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        #[cfg(test)]
        self.resolve_hooks
            .reach(entity, tests::ResolvePoint::Acquired);
        if let Some(known) = self.known(entity) {
            self.retire(entity, &claim);
            #[cfg(test)]
            self.resolve_hooks
                .reach(entity, tests::ResolvePoint::Reused);
            return known;
        }
        let name = entity.to_owned();
        let mut warnings = Vec::new();
        let decode = self.library().decode();
        let baked = (|| {
            #[cfg(test)]
            self.resolve_hooks.bake(entity, tests::ResolvePoint::Bake)?;
            let baked = plane_library(&self.layers, decode, &self.roster, [&name], &mut warnings)?;
            #[cfg(test)]
            self.resolve_hooks
                .bake(entity, tests::ResolvePoint::Baked)?;
            Ok::<_, io::Error>(baked)
        })();
        let mut unusable = self
            .unusable
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.lazy_diagnostics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend(warnings);
        let (shard, refused) = baked.map_err(|error| Unresolved::Refused {
            entity: name.clone(),
            reason: error.to_string(),
        })?;
        unusable.extend(refused);
        #[cfg(test)]
        self.resolve_hooks
            .reach(entity, tests::ResolvePoint::BeforeMerge);
        self.library
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .merge(shard);
        let resolved = resolve_entity(
            std::slice::from_ref(&name),
            |entity| self.library().slot(entity),
            &unusable,
        )
        .and_then(|slot| slot.ok_or(Unresolved::Missing(name)));
        if resolved.is_ok() || unusable.contains_key(entity) {
            self.retire(entity, &claim);
        }
        resolved
    }

    fn known(&self, entity: &str) -> Option<Result<usize, Unresolved>> {
        #[cfg(test)]
        if matches!(
            self.unusable.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ) {
            self.resolve_hooks
                .reach(entity, tests::ResolvePoint::DecisionWaiting);
        }
        if let Some(reason) = self
            .unusable
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(entity)
        {
            return Some(Err(Unresolved::Refused {
                entity: entity.to_owned(),
                reason: reason.clone(),
            }));
        }
        self.library().slot(entity).map(Ok)
    }

    fn retire(&self, entity: &str, claim: &Arc<Mutex<()>>) {
        let mut bakes = self
            .bakes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if bakes
            .get(entity)
            .is_some_and(|current| Arc::ptr_eq(current, claim))
        {
            bakes.remove(entity);
        }
    }

    fn claim(&self, entity: &str) -> Arc<Mutex<()>> {
        Arc::clone(
            self.bakes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entry(entity.to_owned())
                .or_default(),
        )
    }
}

impl AircraftAssets {
    pub fn roster(&self) -> &mesh::Roster {
        &self.roster
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerAircraft {
    pub name: String,
    pub slot: usize,
}

impl AircraftAssets {
    pub fn player(&self, name: &str) -> Result<PlayerAircraft, String> {
        let why = || aircraft_catalogue::refusal(&self.roster, name).map(|refusal| refusal.reason);
        match self.resolve(name) {
            Ok(slot) if self.library().has_geometry(slot) => Ok(PlayerAircraft {
                name: name.to_owned(),
                slot,
            }),
            Ok(_) => {
                self.layers.note_refused(format!("model entity {name}"));
                Err(why().unwrap_or_else(|| "it has no drawable geometry".into()))
            }
            Err(Unresolved::Refused { reason, .. }) => Err(why().unwrap_or(reason)),
            Err(Unresolved::Missing(_)) => Err(why().unwrap_or_else(|| NOT_LOADED.to_owned())),
            Err(Unresolved::Identity | Unresolved::Token(_)) => {
                unreachable!("a chosen name has no wing identity or equipment token")
            }
        }
    }
}

fn first_listings(provinces: &[u32]) -> Vec<u32> {
    let mut seen = BTreeSet::new();
    provinces
        .iter()
        .copied()
        .filter(|province| seen.insert(*province))
        .collect()
}

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

#[derive(Clone, PartialEq)]
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
    fn build(
        start: [f32; 3],
        end: [f32; 3],
        back: [f32; 3],
        takeoff: bool,
        heights: &HeightField<'_>,
        defines: &FlightDefines,
    ) -> Self {
        let distance = norm(sub(end, start));
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
                p[1] = p[1].max(heights.native_height(p[0], p[2]) + defines.curve_elevation);
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum Explode {
    Absent,
    Plays,
}

fn explode(rig: &viewer_models::entity_animation::Rig) -> Explode {
    if rig.nodes.first().is_some_and(|root| {
        root.clips
            .iter()
            .any(|clip| clip.state.name == EXPLODE_STATE)
    }) {
        Explode::Plays
    } else {
        Explode::Absent
    }
}

fn played_at(
    (rig, palette): &ActorSource,
    instance: ModelInstance,
    requests: &Arc<Vec<Request>>,
    at: f32,
) -> viewer_models::entity_animation::Player {
    let mut player = viewer_models::entity_animation::Player::new(rig);
    let mut palette = palette.clone();
    player.evaluate_clock(
        rig,
        instance,
        &Clock {
            time: at,
            phase: None,
            requests: Arc::clone(requests),
        },
        &mut palette,
        &mut Vec::new(),
    );
    player
}

fn played_end(source: &ActorSource, instance: ModelInstance) -> f32 {
    let player = played_at(source, instance, &Arc::new(Vec::new()), 0.0);
    match player.root_play_clip(&source.0) {
        None | Some((_, _, true)) => 0.0,
        Some((timing, age, false)) => timing
            .reaches(timing.length)
            .map_or(f32::INFINITY, |from| (from - age).max(0.0)),
    }
}

fn rig_pose(poses: &[Transform], fps: f32, time: f32) -> Transform {
    let frame = time.max(0.0) * fps;
    let a = &poses[(frame as usize).min(poses.len() - 1)];
    let b = &poses[(frame as usize + 1).min(poses.len() - 1)];
    let t = frame.fract();
    Transform {
        translation: lerp(a.translation, b.translation, t),
        basis: std::array::from_fn(|i| unit(lerp(a.basis[i], b.basis[i], t))),
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Terrain {
    width: u32,
    height: u32,
    column: usize,
    bottom: usize,
    columns: usize,
    values: Vec<u8>,
}

impl Terrain {
    fn capture(heights: &HeightField<'_>, (x0, x1): (f32, f32), (z0, z1): (f32, f32)) -> Self {
        let (width, height) = (heights.width, heights.height);
        let column =
            |x: f32| ((x - 0.5) as i64).clamp(0, i64::from(width.saturating_sub(2))) as usize;
        let bottom =
            |z: f32| ((z - 0.5) as i64).clamp(0, i64::from(height.saturating_sub(2))) as usize;
        let (c0, c1) = (column(x0), (column(x1) + 1).min(width as usize - 1));
        let (b0, b1) = (bottom(z0), (bottom(z1) + 1).min(height as usize - 1));
        let mut values = Vec::with_capacity((c1 - c0 + 1) * (b1 - b0 + 1));
        for b in b0..=b1 {
            let row = height as usize - 1 - b;
            for c in c0..=c1 {
                values.push(heights.values[row * width as usize + c]);
            }
        }
        Self {
            width,
            height,
            column: c0,
            bottom: b0,
            columns: c1 - c0 + 1,
            values,
        }
    }

    fn native_height(&self, x: f32, z: f32) -> f32 {
        let x = x - 0.5;
        let z = z - 0.5;
        let column = (x as i64).clamp(0, i64::from(self.width.saturating_sub(2))) as usize;
        let bottom = (z as i64).clamp(0, i64::from(self.height.saturating_sub(2))) as usize;
        let sample = |column: usize, bottom: usize| {
            let bottom = bottom.min(self.height as usize - 1) - self.bottom;
            let column = column.min(self.width as usize - 1) - self.column;
            f32::from(self.values[bottom * self.columns + column])
        };
        let tx = x - column as f32;
        let tz = z - bottom as f32;
        let a = sample(column, bottom);
        let b = sample(column + 1, bottom);
        let c = sample(column, bottom + 1);
        let d = sample(column + 1, bottom + 1);
        let low = (b - a) * tx + a;
        let high = (d - c) * tx + c;
        (((high - low) * tz + low) * viewer_gpu::province_map::HEIGHT_SCALE)
            .max(viewer_gpu::province_map::WATER_HEIGHT)
    }
}

type Played<'a> = &'a dyn Fn((usize, u32, u32)) -> Option<Option<(f32, f32)>>;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Ground {
    Following,
    Exploding {
        pose: Transform,
        entry: f32,
        hidden: bool,
    },
    Hidden,
}

#[derive(Clone, Debug)]
struct Run {
    initialized: bool,
    flights: std::ops::Range<usize>,
    start: f32,
    cycle: u32,
    grounds: Vec<Ground>,
}

const EASING_FRAME_MS: f32 = 33.333_336;
const EASED_HEADING_MINIMUM: f32 = 0.001;

type FlightKey = (u64, u32, String, usize, [u32; 3], u32);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Eased {
    cycle: u32,
    position: [f32; 3],
    heading: [f32; 3],
    up: [f32; 3],
}

impl Eased {
    fn at(cycle: u32, pose: &Transform) -> Self {
        Self {
            cycle,
            position: pose.translation,
            heading: pose.basis[2],
            up: pose.basis[1],
        }
    }

    fn ease(&mut self, pose: &Transform, frames: f32, step_move: f32, step_turn: f32) {
        let fraction = |value: f32| if value >= 0.0 { value.min(1.0) } else { 0.0 };
        let (moving, turning) = (fraction(frames * step_move), fraction(step_turn * frames));
        let toward = |from: [f32; 3], to: [f32; 3], t: f32| add(mul(sub(to, from), t), from);
        self.position = toward(self.position, pose.translation, moving);
        self.heading = toward(self.heading, pose.basis[2], turning);
        self.up = toward(self.up, pose.basis[1], turning);
        self.heading = if self
            .heading
            .iter()
            .any(|axis| axis.abs() > EASED_HEADING_MINIMUM)
        {
            unit(self.heading)
        } else {
            pose.basis[2]
        };
    }

    fn basis(&self) -> [[f32; 3]; 3] {
        let cross = |a: [f32; 3], b: [f32; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let heading = unit(self.heading);
        let right = unit(cross(self.up, heading));
        let up = unit(cross(heading, right));
        [right, up, heading]
    }
}

fn pose_instance(instance: &mut ModelInstance, position: [f32; 3], basis: [[f32; 3]; 3]) {
    instance.position = position;
    instance.native_basis = basis;
    instance.pitch = (-basis[2][1]).clamp(-1.0, 1.0).asin();
    instance.rotation = (-basis[2][0]).atan2(basis[2][2]);
    instance.roll = basis[0][1].atan2(basis[1][1]);
}

impl Flight {
    fn key(&self) -> FlightKey {
        (
            self.wing,
            self.region,
            self.rig.clone(),
            self.track,
            self.origin.map(f32::to_bits),
            self.yaw.to_bits(),
        )
    }

    fn cycle(&self, duration: f32) -> f32 {
        2.0 * self.group_travel + duration
    }
}

#[derive(Clone, PartialEq)]
struct Flight {
    mission: AirMission,
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
    impact: Option<(u32, [f32; 3], [f32; 4])>,
    lands: bool,
    explode: Explode,
    terrain: Option<Arc<Terrain>>,
}

#[derive(Clone, Default)]
pub struct AircraftPlan {
    flights: Vec<Flight>,
    flight_seeds: Vec<u32>,
    launch_seeds: Vec<u32>,
    runs: Vec<Run>,
    ran_at: Option<f32>,
    finished: Vec<viewer_models::strike_effects::Impact>,
    eased: BTreeMap<FlightKey, Eased>,
    eased_at: Option<f32>,
    launches: Vec<launches::Launch>,
    opened: bool,
    restarts: usize,
    drawn_keys: BTreeSet<FlightKey>,
    snapped: usize,
    pub diagnostics: Vec<String>,
    pub marks: Marks,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Marks {
    pub sources: u64,
    pub src_core: u64,
    pub src_missions: u64,
    pub src_wars: u64,
    pub src_scans: u64,
    pub src_tail: u64,
    pub admission: u64,
    pub generation: u64,
    pub training: u64,
    pub requests: u64,
    pub models: u64,
    pub placement: u64,
    pub wings: usize,
    pub scenarios: usize,
    pub flights_placed: usize,
    pub kept: usize,
    pub flights_kept: usize,
}

#[derive(Default)]
pub struct Kept {
    placed: HashMap<Placing, Vec<Flight>>,
    regions: Option<(ViewIdentity, Arc<Vec<u64>>, Arc<RegionStates>)>,
}

impl Kept {
    fn regions(&mut self, frame: &DecodedFrame, region_provinces: &BTreeMap<u32, Vec<u32>>) -> io::Result<Arc<RegionStates>> {
        let view = &frame.tables[replay_data::rows::TableId::Provinces.index()];
        if let Some((identity, _, held)) = &self.regions
            && identity.is(view)
        {
            viewer_runtime::view_record::add("air scans", "regions kept", 1);
            return Ok(Arc::clone(held));
        }
        let read = Arc::new(RegionStates::states_read(frame)?);
        if let Some((identity, before, held)) = &mut self.regions
            && *before == read
        {
            viewer_runtime::view_record::add("air scans", "regions stand", 1);
            let held = Arc::clone(held);
            (*identity, *before) = (view.identity(), read);
            return Ok(held);
        }
        viewer_runtime::view_record::add("air scans", "regions read", 1);
        let states = Arc::new(RegionStates::read(frame, region_provinces)?);
        self.regions = Some((view.identity(), read, Arc::clone(&states)));
        Ok(states)
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Placing {
    rig: String,
    mission: AirMission,
    region: u32,
    province: u32,
    start: [u32; 3],
    yaw: u32,
    phase: u32,
    actors: [Option<(usize, u64, [u32; 3], bool)>; 2],
}

impl Placing {
    fn of(batch: &Batch<'_>, province: u32, yaw: f32, phase: f32) -> Self {
        let actor = |actor: &Option<Actor>| {
            actor.as_ref().map(|actor| {
                (
                    actor.mesh,
                    actor.wing,
                    actor.start.map(f32::to_bits),
                    actor.explode == Explode::Plays,
                )
            })
        };
        Self {
            rig: batch.key.to_owned(),
            mission: batch.mission,
            region: batch.region,
            province,
            start: batch.start.map(f32::to_bits),
            yaw: yaw.to_bits(),
            phase: phase.to_bits(),
            actors: [actor(&batch.attacker), actor(&batch.defender)],
        }
    }
}

#[derive(Default)]
struct Keeping {
    before: HashMap<Placing, Vec<Flight>>,
    now: HashMap<Placing, Vec<Flight>>,
    fresh: Vec<(Placing, Range<usize>)>,
}

impl Keeping {
    fn take(&mut self, key: &Placing) -> Option<Vec<Flight>> {
        self.before.remove(key).or_else(|| self.now.get(key).cloned())
    }
}

impl Marks {
    pub fn parts(&self) -> [(&'static str, u64); 12] {
        [
            ("sources", self.sources),
            ("src_core", self.src_core),
            ("src_missions", self.src_missions),
            ("src_wars", self.src_wars),
            ("src_scans", self.src_scans),
            ("src_tail", self.src_tail),
            ("admission", self.admission),
            ("generation", self.generation),
            ("training", self.training),
            ("requests", self.requests),
            ("models", self.models),
            ("placement", self.placement),
        ]
    }
}

fn since(started: Instant) -> u64 {
    started.elapsed().as_nanos() as u64
}

const LAUNCH_IMPACT: u64 = 1 << 63;

struct LaunchSources<'a, 'c> {
    frame: &'a DecodedFrame,
    assets: &'a AircraftAssets,
    carriers: &'a CarrierSources<'c>,
    impact_place: &'a dyn Fn(u32) -> Option<([f32; 3], [f32; 4])>,
    clock_speed: f32,
}

#[derive(Default)]
struct LaunchRecords {
    bases: BTreeSet<u32>,
    unstarted: BTreeMap<String, usize>,
    undrawn: BTreeMap<String, usize>,
    places: BTreeMap<u32, launches::ExplosionPlaces>,
    started: usize,
}

fn launch_entity(frame: &DecodedFrame, wing: u64) -> Result<String, String> {
    let variant = aircraft_model::wing_primary(frame, wing)?
        .ok_or("its equipment identity is not recorded")?;
    let definitions = frame
        .complete::<tables::VariantDefinitions>()
        .map_err(|error| error.to_string())?;
    let row = definitions
        .iter()
        .find(|row| row.id == variant)
        .ok_or_else(|| format!("its equipment variant {variant} has no definition"))?;
    let tokens = TokenNames::new(frame).map_err(|error| error.to_string())?;
    let name = tokens
        .name(row.equipment_token)
        .map_err(|error| format!("its equipment type: {error}"))?;
    Ok(format!("{name}_entity"))
}

impl LaunchRecords {
    fn start(
        &mut self,
        request: scenarios::LaunchRequest<'_>,
        sources: &LaunchSources<'_, '_>,
    ) -> Option<launches::Launch> {
        let (wing, base) = (request.wing.wing, request.wing.base);
        let province = match base.state {
            Some(_) => base.province.ok_or("its base's province is not recorded"),
            None => sources.carriers.province(base),
        };
        let province = match province {
            Ok(province) => province,
            Err(reason) => {
                *self.unstarted.entry(reason.to_owned()).or_default() += 1;
                return None;
            }
        };
        if self.bases.contains(&province) {
            return None;
        }
        let entity = match launch_entity(sources.frame, wing.id) {
            Ok(entity) => entity,
            Err(reason) => {
                *self.unstarted.entry(reason).or_default() += 1;
                return None;
            }
        };
        let roster = &sources.assets.roster;
        if !roster.descriptors.entities.contains_key(&entity)
            && !roster.unreadable_names.contains_key(&entity)
        {
            *self
                .unstarted
                .entry(format!(
                    "the content declares no {entity}, so the game spawns nothing"
                ))
                .or_default() += 1;
            return None;
        }
        self.bases.insert(province);
        match self.drawn(request, &entity, province, sources) {
            Ok(launch) => {
                self.started += 1;
                Some(launch)
            }
            Err(reason) => {
                *self.undrawn.entry(reason).or_default() += 1;
                None
            }
        }
    }

    fn drawn(
        &mut self,
        request: scenarios::LaunchRequest<'_>,
        entity: &str,
        province: u32,
        sources: &LaunchSources<'_, '_>,
    ) -> Result<launches::Launch, String> {
        let index = self.started;
        let (wing, base) = (request.wing.wing, request.wing.base);
        let assets = sources.assets;
        let mesh = assets.resolve(entity).map_err(|error| match error {
            Unresolved::Refused { entity, reason } => {
                format!("their entity {entity} is refused: {reason}")
            }
            other => format!("their entity {entity} does not resolve: {other:?}"),
        })?;
        let position = take_off(&assets.launch_content, sources.carriers, base)
            .map_err(|reason| format!("their base has no position: {reason}"))?;
        let max_delay = assets.launch_content.max_delay()?;
        let length =
            assets.launch_lifetime(mesh, launches::Launch::instance_for(index).animation_seed);
        let seed = wing.id as u32 ^ (wing.id >> 32) as u32 ^ province.rotate_left(16);
        let explosions = match request.kind {
            scenarios::LaunchKind::Mission => None,
            scenarios::LaunchKind::Definition => {
                let region = wing
                    .mission_region
                    .ok_or("their mission has no strategic region")?;
                Some(Arc::clone(self.places.entry(region).or_insert_with(|| {
                    Arc::new(
                        assets
                            .region_provinces
                            .get(&region)
                            .into_iter()
                            .flatten()
                            .map(|&province| (province, (sources.impact_place)(province)))
                            .collect(),
                    )
                })))
            }
        };
        Ok(launches::Launch {
            mesh,
            wing: wing.id,
            position,
            spawn: launches::Launch::delay(seed, max_delay, sources.clock_speed),
            length,
            seed,
            explosions,
        })
    }

    fn diagnostics(&self) -> impl Iterator<Item = String> + '_ {
        let unstarted = self.unstarted.iter().map(|(reason, count)| {
            format!("{count} base-equipment launch effects start no record: {reason}")
        });
        let undrawn = self.undrawn.iter().map(|(reason, count)| {
            format!("{count} base-equipment launch effects are not drawn: {reason}")
        });
        let unplaced = self.places.iter().filter_map(|(region, places)| {
            if places.is_empty() {
                return Some(format!(
                    "region {region} lists no province, so its launch effects' explosions have nowhere to play"
                ));
            }
            let unplaced = places.iter().filter(|(_, place)| place.is_none()).count();
            (unplaced > 0).then(|| {
                format!(
                    "region {region} lists {unplaced} provinces with no standstill anchor or painted box; a launch effect's explosion drawn there does not play"
                )
            })
        });
        unstarted.chain(undrawn).chain(unplaced)
    }
}

type RegionMembership = (BTreeMap<u32, Vec<u32>>, BTreeMap<u32, RegionFractions>);

fn flight_requests(
    flight: &Flight,
    rig: &Rig,
    actor: &viewer_models::entity_animation::Rig,
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
    warnings: &mut Vec<content_reader_parser::Diagnostic>,
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
        warnings.push((content_reader_parser::DiagnosticKind::Warning, format!(
            "strategic regions list {skipped} provinces missing from the map definitions; the game skips them"
        )));
    }
    (members, fractions)
}

use viewer_unit_reads::air_reachability::Scans;

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

struct RelationRows<'a> {
    rows: &'a [AirCountryRelationRow],
    links: &'a BTreeMap<u32, u32>,
}

impl RelationRows<'_> {
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

#[derive(Clone, Copy)]
struct Visit<'a> {
    call: usize,
    side: usize,
    own: bool,
    region: u32,
    wing: &'a AirWingRow,
}

fn visit_order<'a>(
    wings: &'a [AirWingRow],
    registrations: &[replay_data::rows::AirRegionalMissionRow],
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

const GROUP_CAPACITY: usize = 50;

#[derive(Default)]
struct Groups {
    region: Option<u32>,
    call: Option<usize>,
    open: bool,
    counts: [[usize; 5]; 2],
}

impl Groups {
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

struct NavalPatrolSources {
    combats: BTreeMap<u32, Vec<(u32, u32)>>,
    sums: BTreeMap<(u32, u32), (i64, i64)>,
}

impl NavalPatrolSources {
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
            if row.is_availability_witness() || row.is_menu() {
                continue;
            }
            let Some(index) = row.summation_index(histories).map_err(invalid)? else {
                continue;
            };
            entries.entry((row.country, row.region)).or_default().push((
                index,
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
        let province = (0..reached.len())
            .map(|step| reached[(start + step) % reached.len()])
            .find(|&province| {
                province != combat && kinds.get(&province) == Some(&ProvinceKind::Sea)
            })
            .unwrap_or(reached[start]);
        (SUCCESS, province)
    }
}

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

#[derive(Default)]
struct Caps {
    counts: BTreeMap<(u32, String), usize>,
}

fn group_index(category: Category) -> usize {
    match category {
        Category::BomberFlight => 0,
        Category::Fighter => 1,
        Category::BaseEquipment => 2,
        Category::Transport => 3,
        Category::Scout => 4,
    }
}

fn effective_country(wing: &AirWingRow) -> u32 {
    if wing.country_override > 0 {
        wing.country_override as u32
    } else {
        wing.owner
    }
}

fn wing_random(wing: u64, region: u32) -> Random {
    Random::new(SEED ^ wing as u32 ^ (wing >> 32) as u32 ^ region.rotate_left(16) ^ 0xffff)
}

fn take_off(
    sites: &launches::Content,
    carriers: &CarrierSources<'_>,
    base: &AirBaseRow,
) -> Result<[f32; 3], &'static str> {
    match base.state {
        Some(state) => sites.site(state, base.kind),
        None => carriers.start(base),
    }
}

struct Placement<'a, 'h> {
    geometry: &'a ProvinceGeometry,
    heights: &'a HeightField<'h>,
    defines: &'a FlightDefines,
}

type ActorSource = (Arc<viewer_models::entity_animation::Rig>, Vec<PaletteJoint>);

struct Batch<'a> {
    wing: u64,
    region: u32,
    mission: AirMission,
    key: &'a str,
    rig: &'a Rig,
    start: [f32; 3],
    attacker: Option<Actor>,
    defender: Option<Actor>,
}

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
                && row.scenario_channels == replay_data::rows::Recorded::Value(Some([0; 7]))
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
            if row.is_availability_witness() || row.is_menu() {
                continue;
            }
            let Some(index) = row.summation_index(histories).map_err(invalid)? else {
                continue;
            };
            let [a, b, c, d, e, f, g] = match row.scenario_channels {
                replay_data::rows::Recorded::Value(Some(values)) => values,
                replay_data::rows::Recorded::Value(None) => {
                    unavailable
                        .entry(row.region)
                        .or_insert("first-window aircraft scenario channels unavailable");
                    continue;
                }
                replay_data::rows::Recorded::NotRecorded => {
                    unavailable
                        .entry(row.region)
                        .or_insert("first-window aircraft scenario channels were not recorded");
                    continue;
                }
            };
            regions
                .entry(row.region)
                .or_default()
                .push((index, [a, b, row.our_fighters_lost_raw, c, d, e, f, g]));
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

#[derive(Clone)]
struct Actor {
    mesh: usize,
    wing: u64,
    start: [f32; 3],
    explode: Explode,
}

impl Batch<'_> {
    fn random(&self, ordinal: usize) -> Random {
        Random::new(
            SEED ^ self.wing as u32
                ^ (self.wing >> 32) as u32
                ^ self.region.rotate_left(16)
                ^ ordinal as u32,
        )
    }
}

#[derive(Default)]
struct TrainingGate {
    participants: bool,
    grouped: BTreeSet<u32>,
    undecided: BTreeSet<u32>,
}

impl TrainingGate {
    fn reach(&mut self, region: u32, group: usize) {
        if group < 2 {
            self.grouped.insert(region);
        }
    }

    fn undecided(&mut self, region: u32, group: Option<usize>) {
        if group.is_none_or(|group| group < 2) {
            self.undecided.insert(region);
        }
    }
}

struct TrainingDraw<'a, B> {
    wing: &'a AirWingRow,
    base: B,
    region: u32,
    formation: usize,
    batches: usize,
}

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
        candidates.sort_by_key(|(wing, _)| wing.pool_ordinal);
        if candidates.len() >= GROUP_CAPACITY && unresolved != 0 {
            uncertain += 1;
        }
        candidates.truncate(GROUP_CAPACITY);
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

fn mix(state: u64, value: u64) -> u64 {
    let mut z = state ^ value.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn claim(taken: &mut BTreeSet<u32>, mut hash: u64) -> u32 {
    loop {
        let seed = hash as u32;
        if taken.insert(seed) {
            return seed;
        }
        hash = mix(hash, 1);
    }
}

fn flight_identity(flight: &Flight) -> u64 {
    let mut hash = mix(0, u64::from(flight.region));
    hash = mix(hash, flight.wing);
    hash = mix(hash, flight.track as u64);
    for byte in flight.rig.as_bytes() {
        hash = mix(hash, u64::from(*byte));
    }
    for bits in flight.origin.map(f32::to_bits) {
        hash = mix(hash, u64::from(bits));
    }
    mix(hash, u64::from(flight.yaw.to_bits()))
}

fn launch_identity(launch: &launches::Launch) -> u64 {
    let mut hash = mix(1, launch.wing);
    hash = mix(hash, launch.mesh as u64);
    for bits in launch.position.map(f32::to_bits) {
        hash = mix(hash, u64::from(bits));
    }
    hash = mix(hash, u64::from(launch.spawn.to_bits()));
    mix(hash, u64::from(launch.seed))
}

impl AircraftPlan {
    fn place(
        &mut self,
        batch: &Batch<'_>,
        province: u32,
        rng: &mut Random,
        map: &Placement<'_, '_>,
        keeping: &mut Keeping,
    ) {
        let (start, rig, region) = (batch.start, batch.rig, batch.region);
        let Some([x, z]) = map.geometry.centroid(province) else {
            self.diagnostics.push(format!(
                "region {region} province {province} has no centroid"
            ));
            return;
        };
        let origin = [x, map.heights.native_height(x, z), z];
        let direction = sub(origin, start);
        let yaw = -direction[0].atan2(direction[2])
            + (10.0 + 30.0 * rng.unit().sqrt()).to_radians()
                * if rng.next() & 1 == 0 { -1.0 } else { 1.0 };
        let phase = rng.unit();
        let key = Placing::of(batch, province, yaw, phase);
        let first = self.flights.len();
        if let Some(group) = keeping.take(&key) {
            self.runs.push(Run {
                initialized: false,
                flights: first..first + group.len(),
                start: 0.0,
                cycle: 0,
                grounds: vec![Ground::Following; group.len()],
            });
            self.flights.extend(group.iter().cloned());
            (self.marks.kept, self.marks.flights_kept) =
                (self.marks.kept + 1, self.marks.flights_kept + group.len());
            keeping.now.insert(key, group);
            return;
        }
        let mut group = Vec::new();
        let mut group_travel = 0.0f32;
        for (track, poses) in rig.tracks.iter().enumerate() {
            let Some(actor) = (if rig.sides[track] == 0 {
                batch.attacker.as_ref()
            } else {
                batch.defender.as_ref()
            }) else {
                continue;
            };
            let lands = rig.lands[track];
            let terrain = (!lands).then(|| {
                let mut x = (f32::INFINITY, f32::NEG_INFINITY);
                let mut z = x;
                for pose in poses {
                    let at = add(origin, rotate(pose.translation, yaw));
                    x = (x.0.min(at[0]), x.1.max(at[0]));
                    z = (z.0.min(at[2]), z.1.max(at[2]));
                }
                Arc::new(Terrain::capture(map.heights, x, z))
            });
            let first = poses[0];
            let last = *poses.last().unwrap();
            let unbound = rig.unbound[track];
            let (begin, end, back_first, back_last) = if unbound.is_some() {
                (origin, origin, [0.0; 3], [0.0; 3])
            } else {
                (
                    add(origin, rotate(first.translation, yaw)),
                    add(origin, rotate(last.translation, yaw)),
                    rotate(first.basis[2], yaw),
                    rotate(last.basis[2], yaw),
                )
            };
            let outgoing = Route::build(
                actor.start,
                begin,
                back_first,
                true,
                map.heights,
                map.defines,
            );
            let incoming =
                Route::build(end, actor.start, back_last, false, map.heights, map.defines);
            let frame = 0.1 * rig.fps;
            let tenth = lerp(
                poses[(frame as usize).min(poses.len() - 1)].translation,
                poses[(frame as usize + 1).min(poses.len() - 1)].translation,
                frame.fract(),
            );
            let speed = unbound.unwrap_or_else(|| norm(sub(tenth, first.translation)) / 0.1);
            let travel = if speed > 0.0 {
                outgoing.length() / speed
            } else {
                1.0
            };
            group_travel = group_travel.max(travel);
            group.push(Flight {
                mission: batch.mission,
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
                impact: None,
                lands,
                explode: actor.explode,
                terrain,
            });
        }
        for f in &mut group {
            f.delay = group_travel - f.travel;
            f.group_travel = group_travel;
        }
        (self.marks.scenarios, self.marks.flights_placed) =
            (self.marks.scenarios + 1, self.marks.flights_placed + group.len());
        keeping.fresh.push((key, first..first + group.len()));
        self.runs.push(Run {
            initialized: false,
            flights: first..first + group.len(),
            start: 0.0,
            cycle: 0,
            grounds: vec![Ground::Following; group.len()],
        });
        self.flights.extend(group);
    }

    #[expect(clippy::too_many_arguments, reason = "a build's inputs are given together")]
    pub fn build(
        frame: &DecodedFrame,
        assets: &AircraftAssets,
        region_of: &BTreeMap<u32, u32>,
        geometry: &ProvinceGeometry,
        heights: &HeightField<'_>,
        ours: u32,
        model: &dyn Fn(u64, u32) -> Result<Option<String>, String>,
        carrier_origin: &dyn Fn(&TaskForceRow) -> Result<[f32; 3], &'static str>,
        impact_place: &dyn Fn(u32) -> Option<([f32; 3], [f32; 4])>,
    ) -> io::Result<Self> {
        Self::make(
            frame,
            assets,
            region_of,
            geometry,
            heights,
            ours,
            model,
            carrier_origin,
            impact_place,
            &mut Kept::default(),
        )
    }

    #[expect(clippy::too_many_arguments, reason = "a build's inputs are given together")]
    pub fn make(
        frame: &DecodedFrame,
        assets: &AircraftAssets,
        region_of: &BTreeMap<u32, u32>,
        geometry: &ProvinceGeometry,
        heights: &HeightField<'_>,
        ours: u32,
        model: &dyn Fn(u64, u32) -> Result<Option<String>, String>,
        carrier_origin: &dyn Fn(&TaskForceRow) -> Result<[f32; 3], &'static str>,
        impact_place: &dyn Fn(u32) -> Option<([f32; 3], [f32; 4])>,
        kept: &mut Kept,
    ) -> io::Result<Self> {
        let regions = kept.regions(frame, &assets.region_provinces)?;
        let mut keeping = Keeping {
            before: std::mem::take(&mut kept.placed),
            now: HashMap::new(),
            fresh: Vec::new(),
        };
        let mut out = Self::default();
        let started = Instant::now();
        let (models, placing) = (Cell::new(0), Cell::new(0));
        let links: BTreeMap<_, _> = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|r| (r.id, r.canonical_slot))
            .collect();
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
        out.marks.src_core = since(started);
        let mut source_lap = Instant::now();
        let mission_countries =
            air_reachability::MissionCountrySources::read(frame, &links, &registry)?;
        out.marks.src_missions = since(source_lap);
        source_lap = Instant::now();
        let wars = Wars::read(frame, &links)?;
        let families = Families::read(frame, &links).ok();
        let fog_families = families
            .clone()
            .and_then(|families| families.with_attaches(frame, &links).ok());
        out.marks.src_wars = since(source_lap);
        source_lap = Instant::now();
        let needs = |bits: u32| {
            wings
                .iter()
                .any(|wing| air_reachability::scanned_bits(wing.mission_mask) & bits != 0)
        };
        let scan_lap = |name: &'static str, started: Instant| viewer_runtime::view_record::add("air scans", name, since(started));
        let scans = Scans {
            tactical: needs(1 << 1)
                .then(|| {
                    let started = Instant::now();
                    let read = TacticalTargetSources::read(frame, &wars);
                    scan_lap("tactical ns", started);
                    read
                })
                .transpose()?,
            port: needs(1 << 8)
                .then(|| {
                    let started = Instant::now();
                    let read = PortTargetSources::read(frame, &wars);
                    scan_lap("port ns", started);
                    read
                })
                .transpose()?,
            strategic: needs(0x1_0008)
                .then(|| {
                    let started = Instant::now();
                    let read = StrategicTargetSources::read(frame, &wars, &regions);
                    scan_lap("strategic ns", started);
                    read
                })
                .transpose()?,
            scout: needs(1 << 14)
                .then(|| {
                    let started = Instant::now();
                    let read = ScoutTargetSources::read(frame, &links, &wars, fog_families.as_ref(), &regions);
                    scan_lap("scout ns", started);
                    read
                })
                .transpose()?,
            naval_combat: needs(0x90)
                .then(|| {
                    let started = Instant::now();
                    let read = NavalCombatTargetSources::read(frame, &links, &wars, families.as_ref());
                    scan_lap("naval combat ns", started);
                    read
                })
                .transpose()?,
            occupants: needs(0x90)
                .then(|| {
                    let started = Instant::now();
                    let read = NavalOccupantSources::read(frame, &wars, &assets.province_kinds);
                    scan_lap("occupants ns", started);
                    read
                })
                .transpose()?,
            mine: needs(AIR_MISSION_LAY_MINES)
                .then(|| {
                    let started = Instant::now();
                    let read = MineTargetSources::read(frame, &links);
                    scan_lap("mine ns", started);
                    read
                })
                .transpose()?,
            fractions: &assets.region_fractions,
            dlc_mask,
            mine_maximum_raw: defines.mine_maximum_raw,
        };
        out.marks.src_scans = since(source_lap);
        source_lap = Instant::now();
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
        let primary = model_inputs::Primary::new(frame);
        let wing_mesh = |wing: u64, raw_country: u32| {
            let chosen = Instant::now();
            let mesh = (|| {
                let variant = primary
                    .get(wing)
                    .map_err(Unresolved::Token)?
                    .ok_or(Unresolved::Identity)?;
                let entity = model(variant, raw_country)
                    .map_err(Unresolved::Token)?
                    .ok_or_else(|| Unresolved::Missing(format!("variant {variant}")))?;
                assets.resolve(&entity)
            })();
            models.set(models.get() + since(chosen));
            mesh
        };
        out.marks.src_tail = since(source_lap);
        out.marks.sources = since(started);
        let mut lap = Instant::now();
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
        out.marks.admission = since(lap);
        out.marks.wings = regional
            .values()
            .flat_map(|groups| groups.arrays.iter().flatten())
            .map(Vec::len)
            .sum();
        lap = Instant::now();
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
        let mut records = LaunchRecords::default();
        let mut unplaced_explosions = 0usize;
        let mut unflown_missiles = 0usize;
        for (region, groups) in regional {
            let mut rng = Random::new(SEED ^ region.rotate_left(16));
            let history = history
                .as_ref()
                .map_err(|reason| *reason)
                .and_then(|history| history.region(region));
            let generated = scenarios::generate(
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
            for request in generated.launches {
                if let Some(launch) = records.start(
                    request,
                    &LaunchSources {
                        frame,
                        assets,
                        carriers: &carriers,
                        impact_place,
                        clock_speed: flight_defines.clock_speed,
                    },
                ) {
                    out.launches.push(launch);
                }
            }
            let draws = generated.draws;
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
                    } else if (candidate.side == 1 && matches!(draw.kind, 0 | 1))
                        || draw.kind == scenarios::MISSILE_KIND
                    {
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
                let start = match take_off(&assets.launch_content, &carriers, candidate.base) {
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
                            explode: explode(assets.library().rig(mesh)),
                        }),
                        Err(error) => {
                            refusals.omit(candidate.wing.id, error, &mut out.diagnostics);
                            None
                        }
                    }
                };
                let attacker = actor(candidate, Ok(start));
                let defender = draw.defender.and_then(|candidate| {
                    actor(
                        candidate,
                        take_off(&assets.launch_content, &carriers, candidate.base),
                    )
                });
                let key = rig_key(scenario_name(draw.kind), outcome);
                let Some(rig) = assets.rigs.get(&key) else {
                    if draw.kind == scenarios::MISSILE_KIND {
                        unflown_missiles += 1;
                    } else {
                        out.diagnostics.push(format!("missing scenario {key}"));
                    }
                    continue;
                };
                let batch = Batch {
                    wing: wing.id,
                    region,
                    mission: scenario_mission(draw.kind),
                    key: &key,
                    rig,
                    start,
                    attacker,
                    defender,
                };
                let placed = out.flights.len();
                let placing_at = Instant::now();
                out.place(&batch, province, &mut rng, &placement, &mut keeping);
                placing.set(placing.get() + since(placing_at));
                if draw.kind == scenarios::MISSILE_KIND {
                    match impact_place(province) {
                        Some((position, bounds)) => {
                            for flight in &mut out.flights[placed..] {
                                flight.impact = Some((province, position, bounds));
                            }
                        }
                        None => unplaced_explosions += 1,
                    }
                }
            }
        }
        out.diagnostics.extend(records.diagnostics());
        let inner = models.get() + placing.get();
        out.marks.generation = since(lap).saturating_sub(inner);
        lap = Instant::now();
        if unflown_missiles > 0 {
            out.diagnostics.push(format!(
                "{unflown_missiles} missile bombing scenarios are not flown and their explosions not drawn: the content's missile_1_bombing rig is refused (its content warning names why)"
            ));
        }
        if unplaced_explosions > 0 {
            out.diagnostics.push(format!(
                "{unplaced_explosions} missile scenarios have no explosion: their province has no standstill anchor or painted box"
            ));
        }
        let draws = training_draws(
            wings,
            |wing| links.get(&wing.owner) == Some(&ours),
            |wing| {
                let base = registry
                    .bases
                    .get(&wing.air_base)
                    .ok_or("its pool base does not resolve")?;
                let province = match base.state {
                    Some(_) => base.province.ok_or("its state base has no province")?,
                    None => carriers.province(base)?,
                };
                Ok(region_of
                    .get(&province)
                    .copied()
                    .map(|region| (region, base)))
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
            let start = match take_off(&assets.launch_content, &carriers, draw.base) {
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
                mission: AirMission::Patrol,
                key: &key,
                rig,
                start,
                attacker: Some(Actor {
                    mesh,
                    wing: wing.id,
                    start,
                    explode: explode(assets.library().rig(mesh)),
                }),
                defender: None,
            };
            for ordinal in 0..draw.batches {
                let placing_at = Instant::now();
                out.place(
                    &batch,
                    province,
                    &mut batch.random(ordinal),
                    &placement,
                    &mut keeping,
                );
                placing.set(placing.get() + since(placing_at));
            }
        }
        out.marks.training = since(lap).saturating_sub(models.get() + placing.get() - inner);
        lap = Instant::now();
        for (key, flights) in keeping.fresh {
            for flight in &mut out.flights[flights.clone()] {
                flight.requests = Arc::new(flight_requests(
                    flight,
                    &assets.rigs[&flight.rig],
                    assets.library().rig(flight.mesh),
                    flight_defines.clock_speed,
                ));
            }
            keeping.now.insert(key, out.flights[flights].to_vec());
        }
        kept.placed = keeping.now;
        out.diagnostics.extend(undecided.diagnostics());
        out.diagnostics.extend(refusals.diagnostics());
        out.marks.requests = since(lap);
        (out.marks.models, out.marks.placement) = (models.get(), placing.get());
        out.assign_seeds(&|mesh, seed| assets.launch_lifetime(mesh, seed));
        Ok(out)
    }

    pub fn bytes(&self) -> usize {
        use std::mem::size_of;
        let flights: usize = self
            .flights
            .iter()
            .map(|flight| {
                size_of::<Flight>()
                    + (flight.outgoing.points.capacity() + flight.incoming.points.capacity()) * 16
                    + flight.rig.capacity()
                    + flight.requests.len() * size_of::<viewer_models::entity_animation::Request>()
                    + flight.terrain.as_ref().map_or(0, |terrain| terrain.values.capacity())
            })
            .sum();
        flights
            + self.runs.iter().map(|run| size_of::<Run>() + run.grounds.capacity()).sum::<usize>()
            + self.launches.capacity() * size_of::<launches::Launch>()
            + self.diagnostics.iter().map(String::capacity).sum::<usize>()
    }

    pub fn counts(&self) -> [usize; 3] {
        [self.flights.len(), self.launches.len(), self.diagnostics.len()]
    }

    pub fn differ(&self, other: &Self) -> (u64, u64, Option<String>) {
        let mut differing = 0;
        let mut first = None;
        let mut note = |what: String| {
            differing += 1;
            first.get_or_insert(what);
        };
        let flight = |f: &Flight| {
            ((f.mission, f.mesh, f.wing, f.region, f.track, f.lands, f.explode), (f.origin.map(f32::to_bits), f.yaw.to_bits(), f.travel.to_bits(), f.delay.to_bits(), f.group_travel.to_bits(), f.phase.to_bits()))
        };
        for (at, (a, b)) in self.flights.iter().zip(&other.flights).enumerate() {
            if flight(a) != flight(b) || a.rig != b.rig || a.outgoing != b.outgoing || a.incoming != b.incoming || a.requests != b.requests || a.impact != b.impact || a.terrain != b.terrain {
                note(format!("flight {at}: wing {} region {} rig {} track {} against wing {} region {} rig {} track {}", a.wing, a.region, a.rig, a.track, b.wing, b.region, b.rig, b.track));
            }
        }
        if self.flights.len() != other.flights.len() {
            note(format!("{} flights against {}", self.flights.len(), other.flights.len()));
        }
        let runs = |plan: &Self| plan.runs.iter().map(|run| (run.flights.clone(), run.grounds.len())).collect::<Vec<_>>();
        if runs(self) != runs(other) {
            note("the scenario runs differ".into());
        }
        if self.launches != other.launches {
            note(format!("{} launches against {}", self.launches.len(), other.launches.len()));
        }
        if self.diagnostics != other.diagnostics {
            let at = self.diagnostics.iter().zip(&other.diagnostics).position(|(a, b)| a != b);
            note(format!("diagnostics: {:?} against {:?}", at.map(|at| &self.diagnostics[at]), at.map(|at| &other.diagnostics[at])));
        }
        let entries = self.flights.len().max(other.flights.len()) + self.launches.len().max(other.launches.len()) + self.diagnostics.len().max(other.diagnostics.len()) + 1;
        (entries as u64, differing, first)
    }

    pub fn missile_explosions(&mut self, from: f32, to: f32) -> Vec<viewer_models::strike_effects::Impact> {
        let mut out = std::mem::take(&mut self.finished);
        if from.is_nan() || to.is_nan() {
            return out;
        }
        let opening = !self.opened && from <= 0.0 && to >= 0.0;
        if to >= 0.0 {
            self.opened = true;
        }
        if opening {
            out.extend(self.launch_explosions(|launch| launch.finished_at_open()));
        }
        let from = if opening { from.max(0.0) } else { from };
        if to > from {
            out.extend(self.launch_explosions(|launch| launch.finished(from, to)));
        }
        out
    }

    pub fn carry_opening(&mut self, previous: &Self) {
        self.opened |= previous.opened;
    }

    fn advance(
        &mut self,
        rigs: &BTreeMap<String, Rig>,
        defines: &FlightDefines,
        band: Option<Band>,
        seconds: f32,
        played: Played<'_>,
    ) {
        let restart = self.ran_at.is_none_or(|last| seconds < last);
        self.ran_at = Some(seconds);
        let clock_speed = defines.clock_speed;
        for run in &mut self.runs {
            let Some(lead) = self.flights.get(run.flights.start) else {
                continue;
            };
            let rig = &rigs[&lead.rig];
            let total = lead.cycle(rig.duration);
            if total.is_nan() || total <= 0.0 {
                continue;
            }
            let elapsed = seconds.max(0.0) * clock_speed + lead.phase * total;
            let reset = |run: &mut Run| run.grounds.fill(Ground::Following);
            if restart || !run.initialized || elapsed < run.start {
                run.initialized = true;
                let cycle = (elapsed / total).floor();
                run.start = cycle * total;
                run.cycle = cycle as u32;
                reset(run);
            }
            let finish = |run: &Run, finished: &mut Vec<viewer_models::strike_effects::Impact>| {
                for index in run.flights.clone() {
                    if let Some((province, position, bounds)) = self.flights[index].impact {
                        finished.push(viewer_models::strike_effects::Impact {
                            id: u64::from(
                                self.flight_seeds
                                    .get(index)
                                    .copied()
                                    .unwrap_or(index as u32),
                            ) << 32
                                | u64::from(run.cycle),
                            province,
                            effect: viewer_models::strike_effects::Effect::MissileExplosion,
                            position,
                            bounds,
                        });
                    }
                }
            };
            let mut time = elapsed - run.start;
            if time >= total {
                let cycles = (time / total).floor();
                run.start += cycles * total;
                run.cycle += cycles as u32;
                time -= cycles * total;
                reset(run);
                finish(run, &mut self.finished);
            }
            let Some(band) = band else {
                continue;
            };
            if time < lead.group_travel || time >= lead.group_travel + rig.duration {
                continue;
            }
            let rig_time = time - lead.group_travel;
            let mut grounded = 0;
            let mut done = 0;
            for (index, ground) in run.flights.clone().zip(run.grounds.iter_mut()) {
                let flight = &self.flights[index];
                let Some(terrain) = flight.terrain.as_ref().filter(|_| !flight.lands) else {
                    continue;
                };
                grounded += 1;
                let local = rig_pose(&rigs[&flight.rig].tracks[flight.track], rig.fps, rig_time);
                let pose = Transform {
                    translation: add(flight.origin, rotate(local.translation, flight.yaw)),
                    basis: local.basis.map(|axis| rotate(axis, flight.yaw)),
                };
                let at = pose.translation;
                let surface = terrain.native_height(at[0], at[2]);
                let on_ground = rig.rockets[flight.track] && at[1] <= surface;
                let in_band = at[1] <= surface + band.offset;
                let is_done = match *ground {
                    Ground::Following if on_ground => true,
                    Ground::Following if in_band => match flight.explode {
                        Explode::Plays => {
                            *ground = Ground::Exploding {
                                pose,
                                entry: (time - flight.delay) / clock_speed,
                                hidden: false,
                            };
                            false
                        }
                        Explode::Absent => {
                            *ground = Ground::Hidden;
                            true
                        }
                    },
                    Ground::Following => false,
                    Ground::Hidden => on_ground || in_band,
                    Ground::Exploding {
                        pose,
                        entry,
                        hidden,
                    } => {
                        let read = hidden
                            || GroundContact::from(band).exploded(played((
                                flight.mesh,
                                self.flight_seeds
                                    .get(index)
                                    .copied()
                                    .unwrap_or(index as u32),
                                run.cycle,
                            )));
                        *ground = Ground::Exploding {
                            pose,
                            entry,
                            hidden: hidden || read,
                        };
                        read
                    }
                };
                done += usize::from(is_done);
            }
            if grounded > 0 && grounded == run.grounds.len() && done == grounded {
                run.cycle += 1;
                run.start = elapsed;
                reset(run);
                finish(run, &mut self.finished);
            }
        }
    }

    fn launch_explosions<'a>(
        &'a self,
        finished: impl Fn(&launches::Launch) -> Option<u32> + 'a,
    ) -> impl Iterator<Item = viewer_models::strike_effects::Impact> + 'a {
        self.launches
            .iter()
            .enumerate()
            .filter_map(move |(index, launch)| {
                launch.explosions.as_ref()?;
                let record = finished(launch)?;
                let (province, position, bounds) = launch.explosion(record)?;
                Some(viewer_models::strike_effects::Impact {
                    id: LAUNCH_IMPACT
                        | u64::from(
                            self.launch_seed(index)
                                ^ launches::Launch::instance_for(0).animation_seed,
                        ) << 32
                        | u64::from(record),
                    province,
                    effect: viewer_models::strike_effects::Effect::MissileExplosion,
                    position,
                    bounds,
                })
            })
    }

    #[cfg(test)]
    pub(crate) fn rocket_explosions_for_tests(
        provinces: &[(u32, [f32; 3])],
        frames: &[f32],
    ) -> Vec<Vec<viewer_models::strike_effects::Impact>> {
        let route = Route {
            points: Vec::new(),
            arcs: Vec::new(),
        };
        let rig = Rig {
            tracks: vec![vec![Transform::IDENTITY; 2]],
            sides: vec![0],
            fps: 1.0,
            duration: 1.0,
            events: Vec::new(),
            speed: 1.0,
            unbound: vec![Some(15.0)],
            lands: vec![false],
            rockets: vec![true],
        };
        let flat = HeightField {
            width: 2,
            height: 2,
            values: &[0; 4],
        };
        let mut plan = Self {
            flights: provinces
                .iter()
                .map(|&(province, anchor)| Flight {
                    mission: AirMission::Missile,
                    mesh: 0,
                    wing: 1,
                    region: 7,
                    rig: rig_key(scenario_name(scenarios::MISSILE_KIND), 0),
                    track: 0,
                    origin: [anchor[0], f32::NEG_INFINITY, anchor[2]],
                    yaw: 0.0,
                    outgoing: route.clone(),
                    incoming: route.clone(),
                    travel: 2.0,
                    delay: 0.0,
                    group_travel: 2.0,
                    phase: 0.0,
                    requests: Arc::default(),
                    impact: Some((province, anchor, [0.0, 0.0, 1.0, 1.0])),
                    lands: false,
                    explode: Explode::Absent,
                    terrain: Some(Arc::new(Terrain::capture(&flat, (0.0, 1.0), (0.0, 1.0)))),
                })
                .collect(),
            runs: (0..provinces.len())
                .map(|index| Run {
                    initialized: false,
                    flights: index..index + 1,
                    start: 0.0,
                    cycle: 0,
                    grounds: vec![Ground::Following],
                })
                .collect(),
            ..Self::default()
        };
        let rigs = BTreeMap::from([(rig_key(scenario_name(scenarios::MISSILE_KIND), 0), rig)]);
        let defines = FlightDefines::fixture();
        frames
            .iter()
            .map(|&frame| {
                plan.advance(
                    &rigs,
                    &defines,
                    Some(Band {
                        offset: 0.0,
                        explosion_delay: 0.45,
                    }),
                    frame,
                    &|_| None,
                );
                std::mem::take(&mut plan.finished)
            })
            .collect()
    }

    fn placed(&self, assets: &AircraftAssets, seconds: f32) -> Vec<(usize, ModelInstance)> {
        let Ok(defines) = &assets.flight_defines else {
            return Vec::new();
        };
        place_flights(&self.flights, &assets.rigs, defines, seconds)
    }

    fn eased_runs(
        &mut self,
        rigs: &BTreeMap<String, Rig>,
        defines: &FlightDefines,
        seconds: f32,
    ) -> Vec<(usize, ModelInstance)> {
        let mut placed = self.placed_runs(rigs, defines, seconds);
        let frames = self
            .eased_at
            .filter(|last| seconds >= *last)
            .map(|last| (seconds - last) * 1000.0 / EASING_FRAME_MS);
        if frames.is_none() {
            self.eased.clear();
        }
        self.eased_at = Some(seconds);
        let mut frozen = BTreeMap::new();
        for run in &self.runs {
            for (index, ground) in run.flights.clone().zip(&run.grounds) {
                if let Ground::Exploding { entry, .. } = ground {
                    frozen.insert(index, *entry);
                }
            }
        }
        let turn = defines.smooth_turn * defines.clock_speed.abs();
        for (index, instance) in &mut placed {
            let flight = &self.flights[*index];
            let pose = Transform {
                translation: instance.position,
                basis: instance.native_basis,
            };
            let cycle = instance.animation_epoch;
            let eased = match (frames, self.eased.get_mut(&flight.key())) {
                (Some(frames), Some(eased)) => {
                    if eased.cycle != cycle {
                        *eased = Eased::at(cycle, &flight.outgoing.at(0.0));
                    }
                    if frozen
                        .get(index)
                        .is_none_or(|entry| *entry == instance.animation_phase)
                    {
                        eased.ease(&pose, frames, defines.smooth_move, turn);
                    }
                    *eased
                }
                _ => {
                    if frames.is_some() && self.drawn_keys.contains(&flight.key()) {
                        self.snapped += 1;
                    }
                    let eased = Eased::at(cycle, &pose);
                    self.eased.insert(flight.key(), eased);
                    eased
                }
            };
            pose_instance(instance, eased.position, eased.basis());
        }
        placed
    }

    pub fn assign_seeds(&mut self, launch_lifetime: &dyn Fn(usize, u32) -> f32) {
        let mut taken = BTreeSet::new();
        self.flight_seeds = self
            .flights
            .iter()
            .map(|flight| claim(&mut taken, flight_identity(flight)))
            .collect();
        self.launch_seeds = Vec::with_capacity(self.launches.len());
        for index in 0..self.launches.len() {
            let seed = claim(&mut taken, launch_identity(&self.launches[index]));
            self.launches[index].length = launch_lifetime(self.launches[index].mesh, seed);
            self.launch_seeds.push(seed);
        }
    }

    fn flight_seed(&self, index: usize) -> u32 {
        self.flight_seeds
            .get(index)
            .copied()
            .unwrap_or(index as u32)
    }

    fn launch_seed(&self, index: usize) -> u32 {
        self.launch_seeds
            .get(index)
            .copied()
            .unwrap_or_else(|| launches::Launch::instance_for(index).animation_seed)
    }

    pub fn carry_presentation(
        &mut self,
        previous: &mut Self,
        continuity: viewer_runtime::presentation::Continuity,
    ) -> io::Result<()> {
        if continuity == viewer_runtime::presentation::Continuity::Reset {
            return Ok(());
        }
        let mut prior: BTreeMap<Vec<FlightKey>, Vec<usize>> = BTreeMap::new();
        for (index, run) in previous.runs.iter().enumerate() {
            prior
                .entry(
                    previous.flights[run.flights.clone()]
                        .iter()
                        .map(Flight::key)
                        .collect(),
                )
                .or_default()
                .push(index);
        }
        let mut retained = BTreeSet::new();
        let mut matched: Vec<Option<usize>> = vec![None; self.runs.len()];
        for (at, run) in self.runs.iter().enumerate() {
            let flights = &self.flights[run.flights.clone()];
            let key = flights.iter().map(Flight::key).collect::<Vec<_>>();
            matched[at] = prior.get_mut(&key).and_then(|candidates| {
                let at = candidates.iter().position(|&index| {
                    flights == &previous.flights[previous.runs[index].flights.clone()]
                })?;
                Some(candidates.remove(at))
            });
        }
        self.restarts = 0;
        for (run, matched) in self.runs.iter_mut().zip(matched) {
            match matched {
                Some(index) => {
                    let old = &previous.runs[index];
                    run.start = old.start;
                    run.cycle = old.cycle;
                    run.initialized = old.initialized;
                    if old.grounds.len() == run.grounds.len() {
                        run.grounds.clone_from(&old.grounds);
                    }
                    retained.extend(self.flights[run.flights.clone()].iter().map(Flight::key));
                }
                None => self.restarts += 1,
            }
        }
        self.ran_at = previous.ran_at;
        self.finished.append(&mut previous.finished);
        self.carry_opening(previous);
        self.carry_easing(previous);
        self.eased.retain(|key, _| retained.contains(key));
        Ok(())
    }

    pub fn restarts(&self) -> usize {
        self.restarts
    }

    pub fn carry_easing(&mut self, previous: &mut Self) {
        self.drawn_keys = std::mem::take(&mut previous.drawn_keys);
        let keys: BTreeSet<FlightKey> = self.flights.iter().map(Flight::key).collect();
        self.eased = std::mem::take(&mut previous.eased);
        self.eased.retain(|key, _| keys.contains(key));
        self.eased_at = previous.eased_at;
    }

    fn placed_runs(
        &self,
        rigs: &BTreeMap<String, Rig>,
        defines: &FlightDefines,
        seconds: f32,
    ) -> Vec<(usize, ModelInstance)> {
        let mut runs = vec![None; self.flights.len()];
        for run in &self.runs {
            for (index, ground) in run.flights.clone().zip(&run.grounds) {
                runs[index] = Some((run.start, run.cycle, *ground));
            }
        }
        let mut placed = place_runs(&self.flights, rigs, defines, seconds, &|index| runs[index]);
        for (index, instance) in &mut placed {
            instance.animation_seed = self.flight_seed(*index);
        }
        placed
    }

    fn launched(&self, seconds: f32) -> Vec<(usize, ModelInstance)> {
        self.launches
            .iter()
            .enumerate()
            .filter_map(|(index, launch)| {
                let mut instance = launch.instance(index, seconds)?;
                instance.animation_seed = self.launch_seed(index);
                Some((launch.mesh, instance))
            })
            .collect()
    }

    pub fn next_launch_impact(
        &self,
        seconds: f32,
        mut visible: impl FnMut([f32; 4]) -> bool,
    ) -> f32 {
        self.launches
            .iter()
            .filter_map(|launch| {
                let places = launch.explosions.as_ref()?;
                places
                    .iter()
                    .any(|(_, place)| place.is_some_and(|(_, bounds)| visible(bounds)))
                    .then(|| {
                        let period = launch.period();
                        let end = launch.spawn + launch.length;
                        let elapsed = seconds.rem_euclid(period);
                        if elapsed < end {
                            end - elapsed
                        } else {
                            period - elapsed + end
                        }
                    })
            })
            .fold(f32::INFINITY, f32::min)
    }

    pub fn next_arrival(
        &self,
        assets: &AircraftAssets,
        view: &viewer_models::motion_view::MotionView,
        seconds: f32,
        shown: impl Fn(AirMission) -> bool,
    ) -> f32 {
        use viewer_models::motion_view::Reach;
        let until = |now: f32, start: f32, end: f32, period: f32| {
            if now > end {
                start + period - now
            } else {
                (start - now).max(0.0)
            }
        };
        let library = assets.library();
        let mut next = f32::INFINITY;
        if shown(AirMission::Missile) {
            for launch in &self.launches {
                if view
                    .passage(
                        launch.position,
                        launch.position,
                        Reach::of(&library.meshes[launch.mesh], 1., [0.; 3]),
                    )
                    .is_some()
                {
                    let period = launch.period();
                    let age = seconds.rem_euclid(period);
                    let start = launch.spawn;
                    next = next.min(until(age, start, start + launch.length, period));
                }
            }
        }
        let Ok(defines) = &assets.flight_defines else {
            return next;
        };
        let rate = defines.clock_speed;
        if rate <= 0.0 {
            return next;
        }
        for run in &self.runs {
            for flight in &self.flights[run.flights.clone()] {
                if !shown(flight.mission) {
                    continue;
                }
                let rig = &assets.rigs[&flight.rig];
                let cycle = flight.cycle(rig.duration);
                let age = seconds * rate + flight.phase * cycle - run.start;
                let mesh = &library.meshes[flight.mesh];
                let mut visit = |start, end| {
                    next = next.min(until(age, start, end, cycle) / rate);
                };
                for (route, start) in [
                    (&flight.outgoing, flight.delay),
                    (&flight.incoming, flight.group_travel + rig.duration),
                ] {
                    if start >= flight.group_travel + rig.duration && !flight.lands {
                        continue;
                    }
                    if route.length() <= 0.0 {
                        continue;
                    }
                    view.times(
                        route
                            .arcs
                            .iter()
                            .zip(&route.points)
                            .map(|(&arc, &position)| {
                                (start + flight.travel * (arc / route.length()), position)
                            }),
                        Reach::of(mesh, defines.scale_min.max(1.0), [0.; 3]),
                        &mut visit,
                    );
                }
                let poses = &rig.tracks[flight.track];
                view.times(
                    poses
                        .iter()
                        .enumerate()
                        .map(|(at, pose)| {
                            (
                                flight.group_travel + at as f32 / rig.fps,
                                add(flight.origin, rotate(pose.translation, flight.yaw)),
                            )
                        })
                        .chain(poses.last().map(|pose| {
                            (
                                flight.group_travel + rig.duration,
                                add(flight.origin, rotate(pose.translation, flight.yaw)),
                            )
                        })),
                    Reach::of(mesh, 1., [0.; 3]),
                    visit,
                );
            }
        }
        next
    }

    pub fn field(
        &mut self,
        assets: &AircraftAssets,
        seconds: f32,
        shown: &dyn Fn(AirMission) -> bool,
        played: Played<'_>,
    ) -> (ModelField, BTreeSet<(usize, u32, u32)>) {
        self.snapped = 0;
        let placed = match &assets.flight_defines {
            Ok(defines) => {
                self.advance(&assets.rigs, defines, assets.band, seconds, played);
                self.eased_runs(&assets.rigs, defines, seconds)
            }
            Err(_) => Vec::new(),
        };
        if viewer_runtime::view_record::on() {
            viewer_runtime::view_record::add("flights", "jumps", self.snapped as u64);
            viewer_runtime::view_record::set("flights", "running", self.runs.len() as u64);
            self.drawn_keys = placed.iter().map(|&(index, _)| self.flights[index].key()).collect();
        }
        let mut entries = BTreeMap::new();
        for run in &self.runs {
            for (index, ground) in run.flights.clone().zip(&run.grounds) {
                if let Ground::Exploding { entry, .. } = ground {
                    entries.insert(index, *entry);
                }
            }
        }
        let launched = self.launched(seconds);
        let mut hidden = hidden_flights(&self.flights, &placed, shown);
        let mut grounded = BTreeSet::new();
        for run in &self.runs {
            for (index, ground) in run.flights.clone().zip(&run.grounds) {
                if matches!(
                    ground,
                    Ground::Hidden | Ground::Exploding { hidden: true, .. }
                ) {
                    grounded.insert(index);
                }
            }
        }
        hidden.extend(
            placed
                .iter()
                .filter(|(index, _)| grounded.contains(index))
                .map(|(index, instance)| {
                    let (seed, identity) = instance.animation_key();
                    (self.flights[*index].mesh, seed, identity)
                }),
        );
        if !shown(AirMission::Missile) {
            hidden.extend(launched.iter().map(|(mesh, instance)| {
                let (seed, identity) = instance.animation_key();
                (*mesh, seed, identity)
            }));
        }
        let mut field = ModelField::build(
            &assets.library(),
            placed
                .iter()
                .map(|(index, instance)| (self.flights[*index].mesh, *instance))
                .chain(launched.iter().copied())
                .collect(),
            128.0,
        );
        let unrequested = Arc::new(Vec::new());
        for (mesh, instance) in launched {
            field.clocks.insert(
                (mesh, instance.animation_seed, instance.animation_epoch),
                Clock {
                    time: instance.animation_phase,
                    phase: None,
                    requests: Arc::clone(&unrequested),
                },
            );
        }
        for (index, instance) in placed {
            let flight = &self.flights[index];
            let requests = match entries.get(&index) {
                Some(&entry) => {
                    let mut requests = (*flight.requests).clone();
                    let after = requests.partition_point(|request| request.time <= entry);
                    requests.insert(
                        after,
                        Request {
                            time: entry,
                            phase_time: None,
                            state: EXPLODE_STATE.into(),
                        },
                    );
                    Arc::new(requests)
                }
                None => Arc::clone(&flight.requests),
            };
            field.clocks.insert(
                (
                    flight.mesh,
                    instance.animation_seed,
                    instance.animation_epoch,
                ),
                Clock {
                    time: instance.animation_phase,
                    phase: None,
                    requests,
                },
            );
        }
        (field, hidden)
    }

    pub fn report(&self, assets: &AircraftAssets) -> Vec<String> {
        let mut lines = self.plan_report(assets);
        lines.extend(
            assets
                .lazy_diagnostics()
                .into_iter()
                .map(|(_, reason)| reason),
        );
        lines
    }

    pub fn plan_report(&self, assets: &AircraftAssets) -> Vec<String> {
        let mut lines = vec![format!(
            "Aircraft: {} generated flight slots, {} at time zero, seed 0x{SEED:08x}; live presentation phase is unrecorded",
            self.flights.len(),
            self.placed(assets, 0.0).len()
        )];
        let explosions: BTreeSet<u32> = self
            .flights
            .iter()
            .filter_map(|flight| flight.impact.map(|(province, ..)| province))
            .collect();
        if !explosions.is_empty() {
            lines.push(format!(
                "Aircraft missile explosions: {} scenarios end at provinces {explosions:?}",
                self.flights
                    .iter()
                    .filter(|flight| flight.impact.is_some())
                    .count()
            ));
        }
        for launch in &self.launches {
            let library = assets.library();
            lines.push(format!(
                "Aircraft launch effect: wing {}, {} at {:.4},{:.4},{:.4}, spawns {:.4}s after its pass, plays {:.4}s, every {:.4}s{}",
                launch.wing,
                library.meshes[launch.mesh].name,
                launch.position[0],
                launch.position[1],
                launch.position[2],
                launch.spawn,
                launch.length,
                launch.period(),
                match &launch.explosions {
                    Some(places) => format!(", explodes in one of {} region provinces", places.len()),
                    None => String::new(),
                }
            ));
        }
        {
            let library = assets.library();
            #[expect(clippy::mutable_key_type, reason = "asset keys cache display data outside equality and ordering")]
            let mut wings = BTreeMap::new();
            for (ordinal, f) in self.flights.iter().enumerate() {
                let entry = wings
                    .entry((
                        f.wing,
                        f.region,
                        &f.rig,
                        library.meshes[f.mesh].asset.clone(),
                    ))
                    .or_insert((ordinal, f.mesh, 0usize));
                entry.2 += 1;
            }
            let mut wings: Vec<_> = wings.into_iter().collect();
            wings.sort_by(|(a, av), (b, bv)| {
                (a.0, a.1, a.2, av.0, &a.3).cmp(&(b.0, b.1, b.2, bv.0, &b.3))
            });
            for ((wing, region, rig, _), (_, mesh, count)) in wings {
                lines.push(format!(
                    "Aircraft wing {wing}, region {region}: {count} slots, {rig}, {}, asset scale {}",
                    library.meshes[mesh].name, library.meshes[mesh].scale
                ));
            }
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
        for key in self.flights.iter().map(|f| &f.rig).collect::<BTreeSet<_>>() {
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

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "aircraft/carrier_tests.rs"]
mod carrier_tests;
