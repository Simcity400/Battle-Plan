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
//! Anchors use integer map coordinates and the game's clamped bilinear terrain
//! sample ([`HeightField::native_height`]), as do the route's minimum elevation
//! and the near-ground band. Orientation
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
//! Each frame the drawn transform eases toward the pose, as the game's does:
//! per elapsed 30 Hz frame the position by `AIRPLANES_SMOOTH_INTERPOLATION_MOVE`
//! and the heading and up axis by `AIRPLANES_SMOOTH_INTERPOLATION_TURN` times
//! the clock's speed; a new cycle starts on its take-off route's first point.
//! The band and the runs read the pose, not the eased transform ([`Eased`]).
//! Every state base takes off where the game's base position finder puts it
//! ([`viewer_foundation::map::building_facts::SiteSlots::base`]).
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
//!
//! Base-equipment wings follow the scouts, side A then side B: each yields
//! `max(1, count / MISSILES_DIVISION_FACTOR)` missile bombing scenarios to its
//! mission's last target, checking `MAX_MISSILE_BOMBING_SCENARIOS` before
//! each wing, unless its mission is bit 17 or its pool definition holds the
//! launch flag (AIR_FLAGS bit 33): those start a launch effect at their base
//! instead ([`launches`]), whose entity plays at the base and, for the launch
//! flag, ends in a missile explosion in its mission region. A missile scenario
//! that finishes plays the content's missile explosion entity in its
//! province's effect slot ([`super::strike_effects`]). A rocket site or gun
//! emplacement takes off where the game's base position finder puts it
//! ([`launches`]).
//!
//! The game's scenario analysis gives no landing route to the rocket, to
//! kamikaze fighters that succeed, to the defending slots of a successful
//! interception or dogfight and to the attacking slots of a lost dogfight
//! ([`lands`]). Each frame of the rig such an actor meets the near-ground
//! band ([`grounding`]): the rocket is done at or below the terrain; at or
//! below the terrain plus `AIRPLANES_GROUND_COLLISION_OFFSET_Y` an actor
//! plays its entity's `explode` state and stays where it is, done and hidden
//! once that state's played position reaches
//! `AIRPLANES_GROUND_EXPLOSION_TIME_DELAY`, or, when its entity declares no
//! `explode` state, is hidden and done at once. The scenario ends when every
//! actor is done in the same frame, which skips the landing; otherwise these
//! actors are hidden for it. A scenario actor whose attachment node the rig
//! lacks (vanilla's v2 rocket has only `joint1`) is logged by the game and
//! rides the scenario entity's root: it flies its take-off to the rig origin
//! without a heading, at `NAirGfx.ROCKET_SPEED` for the rocket and one for
//! other slots, and stands there; a rocket there is on the terrain and ends
//! the scenario as the rig starts. The band is tested at the viewer's own
//! frames on the pose each actor draws then, as the game tests it at its
//! frames, hidden missions included ([`AircraftPlan::field`]); an exploding
//! actor is done once its own shared player reads the delay, a pdxmesh root
//! reading the clip it played before when `explode` plays no animation, as
//! the game's entity does. A cycle that ends early starts its next take-off
//! on that frame.

#[cfg(test)]
use crate::map::aircraft_events::callback;
use crate::map::aircraft_events::scenario_side_requests;
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
use viewer_foundation::map::aircraft_model;
use viewer_foundation::map::camera::Camera;
use viewer_foundation::table_access::{TokenKeys, TokenNames};

use engine_air_history::air_kamikaze::window_sum;
mod launches;
mod model_inputs;
mod scenarios;
use installed_content_core::{
    ResolvedLayers, defines::Defines, map::MapData, map::ProvinceKind, mesh, mesh::Transform,
};
use installed_content_equipment_graphics::equipment_graphics::aircraft as aircraft_catalogue;
use installed_content_image::native::Decode;
use installed_content_regions::regions::StrategicRegion;
pub use model_inputs::{KeptModels, ModelMemo, ModelsMade, Selected};
use replay_data::{
    rows::AIR_MISSION_LAY_MINES, rows::AIR_MISSION_NAVAL_PATROL, rows::AIR_MISSION_TRAINING,
    rows::AIR_MISSION_ZERO_PLANE_FLAGS, rows::AirBaseRow, rows::AirCountryRelationRow,
    rows::AirMissionInputRow, rows::AirWingRow, rows::ShipRow, rows::TaskForceRow, rows::tables,
};
use replay_frame::frame::{DecodedFrame, ViewIdentity};
use scenarios::{RegionGroups, ScenarioWing};

use {
    crate::map::entity_animation::Clock, crate::map::entity_animation::Request,
    crate::map::models::ModelField, crate::map::models::ModelInstance,
    crate::map::models::ModelLibrary, crate::map::models::PaletteJoint,
    viewer_foundation::map::air_participants::AirParticipants,
    viewer_foundation::map::air_participants::CountryRelations,
    viewer_foundation::map::air_participants::CountryVectors,
    viewer_foundation::map::air_participants::ordered_participants,
    viewer_foundation::map::air_reachability, viewer_foundation::map::air_reachability::Category,
    viewer_foundation::map::air_reachability::Causes,
    viewer_foundation::map::air_reachability::MineTargetSources,
    viewer_foundation::map::air_reachability::MissionCountry,
    viewer_foundation::map::air_reachability::NavalCombatTargetSources,
    viewer_foundation::map::air_reachability::NavalOccupantSources,
    viewer_foundation::map::air_reachability::PortTargetSources,
    viewer_foundation::map::air_reachability::RegionFractions,
    viewer_foundation::map::air_reachability::RegionStates,
    viewer_foundation::map::air_reachability::ScoutTargetSources,
    viewer_foundation::map::air_reachability::StrategicTargetSources,
    viewer_foundation::map::air_reachability::TacticalTargetSources,
    viewer_foundation::map::air_reachability::TargetAdmission,
    viewer_foundation::map::air_reachability::Wars,
    viewer_foundation::map::air_registry_sources::AirRegistrySources,
    viewer_foundation::map::aircraft_count::SortieDefines,
    viewer_foundation::map::aircraft_count::SortieSources,
    viewer_foundation::map::province_geometry::ProvinceGeometry,
    viewer_foundation::map::province_map::HeightField, viewer_foundation::map::tables::Available,
    viewer_foundation::map::tables::core, viewer_foundation::map::tables::rows,
    viewer_foundation::map::visibility::Families,
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
    /// Base-equipment wings: `MISSILES_DIVISION_FACTOR` and
    /// `MAX_MISSILE_BOMBING_SCENARIOS`; the kind has no triple. Only the
    /// base-equipment loop reads them, so a missing or zero factor refuses
    /// that loop alone ([`scenarios`]).
    missile: Result<FamilyDefines, &'static str>,
    /// `AIRPLANES_ANIMATION_GLOBAL_SPEED_PER_GAMESPEED[0]`, the paused speed
    clock_speed: f32,
    curve_density: f32,
    curve_extrapolation: f32,
    curve_elevation: f32,
    scale_takeoff: f32,
    scale_landing: f32,
    scale_min: f32,
    /// `AIRPLANES_SMOOTH_INTERPOLATION_MOVE` and `_TURN`: how far, per
    /// elapsed 30 Hz frame, an actor's drawn transform eases toward its pose.
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
                // The game divides by it.
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
        // The curve's sample count divides by the density.
        if out.curve_density.is_nan() || out.curve_density <= 0.0 {
            return Err(invalid_data(
                "NDefines.NAirGfx.AIRPLANES_CURVE_POINT_DENSITY must be positive".into(),
            ));
        }
        Ok(out)
    }

    /// Defines for tests: one-plane families, unit clock and curves.
    #[cfg(test)]
    fn for_tests() -> Self {
        let sizing = FamilyDefines {
            factor: 1,
            triple: 3,
            limit: 2,
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
            fire_threshold: 300_000,
            carpet_threshold: 200_000,
            clock_speed: 1.0,
            curve_density: 2.0,
            curve_extrapolation: 20.0,
            curve_elevation: 4.0,
            scale_takeoff: 0.1,
            scale_landing: 0.9,
            scale_min: 0.1,
            smooth_move: 0.13,
            smooth_turn: 0.12,
        }
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

/// What a plane meeting the ground does, from the content's `NAirGfx`:
/// `AIRPLANES_GROUND_COLLISION_OFFSET_Y` is the height above the surface at
/// which it meets it, and `AIRPLANES_GROUND_EXPLOSION_TIME_DELAY` the seconds
/// its explosion plays before the entity is removed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundContact {
    pub offset: f32,
    pub explosion: f32,
}

impl GroundContact {
    /// The near-ground band the content's flights meet ([`Band`]), read once for both.
    pub fn load(defines: &Defines) -> Result<Self, &'static str> {
        Band::load(defines).map(Self::from)
    }

    /// Whether an exploding actor has played the explosion's delay, as its own
    /// player reads it ([`crate::map::models::ModelPass::root_play_position`]):
    /// its root's play position, at its clip's authored speed and start, has
    /// reached the delay; a root that plays no animation reads zero against it.
    /// Before its player exists it has not.
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
pub fn draws(camera: &Camera) -> bool {
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
    /// Per track, the speed of an actor whose attachment node the scenario
    /// entity lacks: the game's analysis leaves its start and end poses at
    /// zero and its speed at the slot's default, and at run time the actor
    /// takes the entity's own root transform ([`scenario_tracks`]).
    unbound: Vec<Option<f32>>,
    /// Per track, whether the game's scenario analysis gives the actor a
    /// landing route ([`lands`]). An actor without one meets the near-ground
    /// band during the rig ([`grounding`]) and is hidden for the landing.
    lands: Vec<bool>,
    /// Per track, whether the actor is the rocket slot, which ends where its
    /// pose reaches the terrain.
    rockets: Vec<bool>,
}

/// The default speed of a scenario actor slot whose attachment node is
/// absent: `NAirGfx.ROCKET_SPEED` for the rocket, one for every other slot.
const UNBOUND_SPEED: f32 = 1.0;

/// The outcome whose attacking slots lose a dogfight (`fail`).
const FAIL: u8 = 3;

/// Whether the game's scenario analysis builds a landing route for a slot on
/// `side` of scenario `kind` playing `outcome`. It builds none for the
/// rocket, for kamikaze fighters that succeed, for the defending slots of a
/// successful interception or dogfight and for the attacking slots of a lost
/// dogfight; every other slot flies home.
fn lands(kind: u8, outcome: u8, side: u8) -> bool {
    !match kind {
        scenarios::MISSILE_KIND => outcome == 0,
        16 | 17 => outcome == SUCCESS,
        12 | 18 => side == 1 && outcome == SUCCESS,
        13 | 14 => (side == 0 && outcome == FAIL) || (side == 1 && outcome == SUCCESS),
        _ => false,
    }
}

/// The state the game requests on an actor that reaches the near-ground band.
const EXPLODE_STATE: &str = "explode";

/// The near-ground band an actor without a landing route meets:
/// `NAirGfx.AIRPLANES_GROUND_COLLISION_OFFSET_Y` above the terrain, and
/// `NAirGfx.AIRPLANES_GROUND_EXPLOSION_TIME_DELAY` of played `explode` state
/// before the actor is hidden.
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
    /// Candidate entities the game would find that cannot be drawn here,
    /// with the reason.
    unusable: Mutex<BTreeMap<String, String>>,
    // Cold resolves of the same entity share one bake; unrelated entities never wait on it.
    bakes: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
    #[cfg(test)]
    resolve_hooks: tests::ResolveHooks,
    /// Asset-load diagnostics from successful or refused lazy bakes.
    lazy_diagnostics: Mutex<Vec<installed_content_core::Diagnostic>>,
    /// Equipment names by the lexer's token identity.
    keys: TokenKeys,
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
    /// The near-ground band; a rig that needs it exists only when it reads.
    band: Option<Band>,
    /// What launch effects and rocket-site and gun-emplacement take-offs
    /// read ([`launches`]).
    launch_content: launches::Content,
    /// How a plane meets the ground, for the flying mode's aircraft.
    ground: Result<GroundContact, String>,
}

/// The exact suffix ladder under each available prefix. Missing recorded
/// override levels are reported, never substituted with a graphical culture.
#[cfg(test)]
pub(crate) fn candidates(tag: &str, name: &str, archetype: &str, sprite: &str) -> Vec<String> {
    aircraft_model::candidates("", tag, name, archetype, sprite)
}

const NOT_LOADED: &str = "its mesh did not load";

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
/// ([`mesh::Roster::refused_declaration`]). A refused entity still exists for
/// the engine's name lookup, so its reason stops resolution rather than
/// skipping it; an entity whose mesh does not load is kept too. When every
/// candidate is refused the library is empty and every reason is kept.
fn plane_library<'a>(
    layers: &ResolvedLayers,
    decode: Decode,
    roster: &mesh::Roster,
    declared: impl IntoIterator<Item = &'a String>,
    warnings: &mut Vec<installed_content_core::Diagnostic>,
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
        warnings.push((installed_content_core::DiagnosticKind::Refused, format!(
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
    // An entity the visual mods' descriptors leave refused, though the recording's own do not,
    // is drawn from the recording; every other refusal is recorded by name for the map's
    // fallback.
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

/// A scenario entity's posed mesh source, read from the file its pdxmesh
/// names; the reason names the file when it does not read.
fn scenario_source(
    layers: &ResolvedLayers,
    descriptors: &mesh::Descriptors,
    entity: &str,
    definition: &mesh::MeshDefinition,
) -> Result<crate::map::mesh_pose::Source, String> {
    let unreadable =
        |error: io::Error| format!("its mesh file {} does not read: {error}", definition.file);
    let tree = mesh::parse_in(layers, &definition.file).map_err(unreadable)?;
    let skeleton = mesh::native_skeleton(&tree, definition.scale).map_err(unreadable)?;
    Ok(crate::map::mesh_pose::Source {
        pdxmesh: true,
        binary_locators: crate::map::mesh_pose::binary_locators(&tree, &skeleton, definition.scale)
            .map_err(unreadable)?,
        // Registration keeps duplicate names in order and the cloned
        // parent's mesh identity.
        entity_locators: crate::map::mesh_pose::entity_locators(
            &descriptors.entities[entity].locators,
            &skeleton,
            Some(&definition.name),
        ),
        skeleton,
        mesh_scale: definition.scale,
        entity_scale: descriptors.resolve_entity(entity).unwrap().scale,
        frozen: None,
        game_data: None,
    })
}

/// The first refused declaration a scenario rig uses: the scenario entity,
/// its mesh and the animation of the state it plays.
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

/// The state a scenario entity plays for `outcome`. A default-outcome
/// scenario entity that declares no `default` state stays in its own default
/// state, as `missile_1_bombing_entity` does in its `idle`.
fn scenario_state<'a>(descriptors: &'a mesh::Descriptors, entity: &str, outcome: u8) -> &'a str {
    let name = state_name(outcome);
    match descriptors.inherit(entity) {
        Some(inherited) if outcome == 0 && !inherited.states.contains_key(name) => {
            inherited.default_state.unwrap_or(mesh::DEFAULT_STATE)
        }
        _ => name,
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
        15 => "missile_1_bombing",
        16 => "fighter_1_kamikaze",
        17 => "fighter_3_kamikaze",
        18 => "fighter_1_vs_transport_1",
        19 => "transport_1_supplying",
        20 => "transport_3_supplying",
        _ => unreachable!("conventional aircraft scenario kind"),
    }
}

/// The mission family a viewer hides a scenario's aircraft with.
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

/// The flights in the air at `seconds`, each by its index among every flight. Every flight keeps
/// its own clock, read from `seconds` and its phase alone, and its animation seed, whatever the
/// display hides.
fn place_flights(
    flights: &[Flight],
    rigs: &BTreeMap<String, Rig>,
    defines: &FlightDefines,
    seconds: f32,
) -> Vec<(usize, ModelInstance)> {
    place_runs(flights, rigs, defines, seconds, &|_| None)
}

/// [`place_flights`] where `run` gives a flight's cycle start, number and
/// ground state on the viewer's frames; without one its cycles follow its
/// phase and it follows its node.
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
            // A rootless rocket based at its target has nothing to show.
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
                // A hidden actor stays placed, so its player keeps stepping
                // ([`AircraftPlan::field`] hides it).
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
                // An actor without a landing route is hidden for the landing.
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
                    // Preserve the rig/route basis during matrix upload; Euler conversion
                    // above is only the retained port's instance declaration.
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

/// The animation keys of the placed flights whose mission is not `shown`.
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
        15 => single("rocket"),
        16 => single("fighter_1"),
        _ => unreachable!("conventional aircraft scenario kind"),
    }
}

/// Resolve each locator once, then
/// sample the shared native source-order palette. Animation joint names do
/// not rebind the skeleton; no content-specific alias is needed. A bone the
/// entity does not hold is logged by the game, which then reads the scenario
/// entity's own root transform for that actor: its track is the entity
/// scale at the rig origin, and it is reported in the second list.
fn scenario_tracks(
    source: &crate::map::mesh_pose::Source,
    animation: &mesh::Animation,
    bones: &[(u8, String)],
) -> io::Result<(Vec<Vec<Transform>>, Vec<bool>)> {
    use {crate::map::mesh_pose, crate::map::mesh_pose::LookupMode};
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

/// The rig of scenario `entity` read from `layers` with its `roster`, or `None` with why in
/// `warnings`.
fn scenario_flown(
    layers: &ResolvedLayers,
    roster: &mesh::Roster,
    entity: &str,
    (kind, outcome): (u8, u8),
    bones: &[(u8, String)],
    defines: (&Result<f32, &'static str>, &Result<Band, &'static str>),
    warnings: &mut Vec<installed_content_core::Diagnostic>,
) -> Option<Rig> {
    let mut refuse = |reason: String| {
        warnings.push((
            installed_content_core::DiagnosticKind::Refused,
            format!("Aircraft scenario {entity} is refused and not flown: {reason}"),
        ));
    };
    let descriptors = &roster.descriptors;
    let state = scenario_state(descriptors, entity, outcome);
    // The engine's choice between duplicate or out-of-order
    // declarations is unverified, so a scenario using one is refused.
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
    // A mesh or animation the content names but that does not read
    // refuses this rig alone, as the plane library refuses an entity
    // it cannot bake: the map opens and the scenario is not flown.
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

/// What `fly` builds for scenario `entity` from the stack it names (`true` for the recorded
/// layers below the visual mods): from the shown stack, or, when that refuses it and
/// `below` holds recorded layers, from those, named in `layers`' visual mod refusals. One
/// neither builds is refused by a stable name for the map's fallback
/// ([`ResolvedLayers::note_refused`]). `warnings` takes the notes of the attempt that is kept.
fn shown_or_recorded<T>(
    layers: &ResolvedLayers,
    entity: &str,
    below: bool,
    mut fly: impl FnMut(bool, &mut Vec<installed_content_core::Diagnostic>) -> Option<T>,
    warnings: &mut Vec<installed_content_core::Diagnostic>,
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
                .find(|(kind, _)| *kind == installed_content_core::DiagnosticKind::Refused)
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

/// A scenario's rig from its entity's attachment tracks, or why it is not
/// flown. An actor slot without its node rides the scenario entity's root
/// at the slot's default speed: `NAirGfx.ROCKET_SPEED` for the rocket, which
/// only that case reads. A rig with an actor that does not land needs the
/// near-ground band's defines ([`Band`]), which only that case reads.
#[allow(clippy::too_many_arguments)]
fn scenario_rig(
    entity: &str,
    (kind, outcome): (u8, u8),
    bones: &[(u8, String)],
    (source, anim): (&crate::map::mesh_pose::Source, &mesh::Animation),
    speed: f32,
    events: Vec<mesh::EntityEvent>,
    (rocket_speed, band): (&Result<f32, &'static str>, &Result<Band, &'static str>),
    warnings: &mut Vec<installed_content_core::Diagnostic>,
) -> Option<Rig> {
    let mut refuse = |reason: String| {
        warnings.push((
            installed_content_core::DiagnosticKind::Refused,
            format!("Aircraft scenario {entity} is refused and not flown: {reason}"),
        ));
        None
    };
    let (tracks, absent) = match scenario_tracks(source, anim, bones) {
        Ok(tracks) if anim.samples >= 2 => tracks,
        Ok(_) => return refuse("it has fewer than two animation samples".into()),
        Err(reason) => return refuse(format!("its attachment tracks fail: {reason}")),
    };
    // The state's `animation_speed` plays the track faster.
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
    // Only an actor without its node rides the root at the fallback speed,
    // so only an unbound rocket needs the rocket speed.
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
                installed_content_core::DiagnosticKind::Logged,
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

    pub fn lazy_diagnostics(&self) -> Vec<installed_content_core::Diagnostic> {
        self.lazy_diagnostics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// What the prepared content refuses when a flight meets it: entities
    /// that cannot be drawn, equipment names one token under the lexer's
    /// fold, and the sortie and flight defines.
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
                .filter(|(kind, _)| *kind == installed_content_core::DiagnosticKind::Refused)
                .map(|(_, reason)| reason),
        );
        refusals
    }

    /// `defines` are the recorded defines, whose operands the sorties count with, and the
    /// graphics defines, which the flights, bands, rockets and launches read, each value falling
    /// back to the recorded one ([`ResolvedLayers::graphics_define`]).
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        map: &MapData,
        (defines, graphics): (&Defines, &Defines),
        regions: &BTreeMap<u32, StrategicRegion>,
        warnings: &mut Vec<installed_content_core::Diagnostic>,
    ) -> io::Result<Self> {
        // The content's equipment database, `duplicate_archetypes` copies
        // included: the archetype is resolved as the definitions are read
        // (a `parent` passes its archetype on), and an `archetype` read
        // copies the archetype's sprite while the type's own is empty
        // A duplicate's own sprite replaces its copies'
        let database = installed_content_core::equipment::Database::load_complete(layers)?;
        let text = |bytes: &[u8], what: &str| {
            installed_content_core::parser::text(bytes)
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
        // Scenario and outcome catalogs provide authored
        // independent and paired rigs, including both callback sides.
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
        // A rig the visual mods break (an animation or mesh of theirs that does not read) is
        // flown from the recorded layers below them, as the plane library bakes an entity.
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
            sortie_defines: viewer_foundation::map::aircraft_count::load_defines(defines)
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

    /// How a plane meets the ground ([`GroundContact`]), or why the content's
    /// defines for it do not read.
    pub fn ground_contact(&self) -> Result<GroundContact, &str> {
        self.ground.as_ref().copied().map_err(String::as_str)
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
        // Retired claims can still have waiters, so every holder re-reads the committed decision.
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
        // Usable actors can still omit attachments; preserve warning kinds and occurrences.
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

    // A remembered refusal takes precedence over a library slot, as in the cold resolve.
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

    /// The claim `entity`'s first bake holds ([`Self::resolve`]).
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
    /// The registered descriptor roster the aircraft bake from.
    pub fn roster(&self) -> &mesh::Roster {
        &self.roster
    }
}

/// A chosen aircraft baked for flying: its slot in [`AircraftAssets::library`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerAircraft {
    /// The entity or bare mesh name it was chosen by.
    pub name: String,
    pub slot: usize,
}

impl AircraftAssets {
    /// Bakes a chosen entity or bare mesh on first use, through the same
    /// roster and checks a recorded flight's entity meets, and returns its
    /// library slot, or why it cannot be flown. The plane resolver admits declarations,
    /// including recorded fallbacks, before files are read. Fly also needs geometry in
    /// that admitted bake; other flights can keep meshless actors' events. A refused name
    /// keeps the catalogue's reason where it has one, or the file load error, and stays
    /// refused. It reads files: call it off the UI thread.
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
            // The game's clamped bilinear terrain sample, as for the anchor.
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

/// Whether an actor's entity declares an `explode` state on its root: one
/// that does plays it in the near-ground band; one that does not is hidden
/// there at once.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Explode {
    Absent,
    Plays,
}

fn explode(rig: &crate::map::entity_animation::Rig) -> Explode {
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

/// An entity's own player for `instance` on `requests`, evaluated at
/// entity time `at`.
fn played_at(
    (rig, palette): &ActorSource,
    instance: ModelInstance,
    requests: &Arc<Vec<Request>>,
    at: f32,
) -> crate::map::entity_animation::Player {
    let mut player = crate::map::entity_animation::Player::new(rig);
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

/// Entity seconds after it spawns until a launch entity's own player, in
/// its default state, reads its root clip's length, where the game removes
/// its record: at once when the root plays no animation, never (infinite)
/// when the position never gets there.
fn played_end(source: &ActorSource, instance: ModelInstance) -> f32 {
    let player = played_at(source, instance, &Arc::new(Vec::new()), 0.0);
    match player.root_play_clip(&source.0) {
        None | Some((_, _, true)) => 0.0,
        Some((timing, age, false)) => timing
            .reaches(timing.length)
            .map_or(f32::INFINITY, |from| (from - age).max(0.0)),
    }
}

/// An actor's pose at rig time `time` relative to its rig origin, as it is
/// drawn: between samples the translation is interpolated and each basis
/// axis interpolated and renormalized.
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

/// The heightmap pixels an actor without a landing route can stand over
/// during its rig, so its band is tested at the viewer's frames on exactly
/// the game's clamped bilinear terrain ([`HeightField::native_height`]).
#[derive(Clone, Debug, PartialEq)]
struct Terrain {
    width: u32,
    height: u32,
    column: usize,
    bottom: usize,
    columns: usize,
    /// The window's bytes, bottom-up from `bottom`, `columns` wide.
    values: Vec<u8>,
}

impl Terrain {
    /// The pixels the sampler reads anywhere over `[x0, x1] x [z0, z1]`.
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

    /// [`HeightField::native_height`] over the captured pixels.
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
        (((high - low) * tz + low) * viewer_foundation::map::province_map::HEIGHT_SCALE)
            .max(viewer_foundation::map::province_map::WATER_HEIGHT)
    }
}

/// What an actor's own player reads, by its animation key, as the previous
/// frame left it ([`crate::map::models::ModelPass::root_play_position`]).
type Played<'a> = &'a dyn Fn((usize, u32, u32)) -> Option<Option<(f32, f32)>>;

/// Where an actor without a landing route stands in the near-ground band
/// this cycle, as the game's update leaves it frame by frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ground {
    /// It follows its node.
    Following,
    /// It entered the band, requested `explode` at entity time `entry` and
    /// stays at `pose`; `hidden` once its player has read the band's delay.
    Exploding {
        pose: Transform,
        entry: f32,
        hidden: bool,
    },
    /// It entered the band without an `explode` state: hidden for good,
    /// still following its node.
    Hidden,
}

/// One placed scenario on the viewer's frames: its flights, the aircraft
/// clock time its current cycle started, the cycle's number and each
/// flight's ground state. A cycle ends after its landing, or on the frame
/// every actor's update reports it done.
#[derive(Clone, Debug)]
struct Run {
    initialized: bool,
    flights: std::ops::Range<usize>,
    start: f32,
    cycle: u32,
    grounds: Vec<Ground>,
}

/// The game's frame period the easing counts frames by, in milliseconds.
const EASING_FRAME_MS: f32 = 33.333_336;
/// A heading this close to zero on every axis is replaced by its target.
const EASED_HEADING_MINIMUM: f32 = 0.001;

/// A flight's identity across hourly plans: its wing, region, rig, track,
/// anchor and heading.
type FlightKey = (u64, u32, String, usize, [u32; 3], u32);

/// An actor's drawn transform: where it is, its heading and its up axis,
/// in the scenario cycle it belongs to.
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

    /// One frame of `frames` 30 Hz frames toward `pose`, as the game eases
    /// an actor each frame: the position by `move` per frame and the heading
    /// and up axis by `turn` per frame, each step at most the whole way; a
    /// heading left within the minimum of zero takes the target's.
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

    /// The basis the game builds from the heading and up axis.
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

/// The instance's rotation fields for a drawn transform.
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

    /// One scenario cycle: the take-off, the rig of `duration` and the
    /// landing. A cycle can end sooner on a frame where every actor of a
    /// scenario none of whose actors lands is done ([`Run`]).
    fn cycle(&self, duration: f32) -> f32 {
        2.0 * self.group_travel + duration
    }
}

#[derive(Clone, PartialEq)]
struct Flight {
    /// The family a viewer can hide this flight's aircraft with.
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
    /// A missile bombing scenario's province, standstill anchor and
    /// admission box, where its explosion plays when the scenario finishes.
    impact: Option<(u32, [f32; 3], [f32; 4])>,
    /// Whether the actor flies the landing route; one that does not is
    /// hidden for the landing and meets the near-ground band.
    lands: bool,
    /// Whether its entity plays `explode` in the band.
    explode: Explode,
    /// The terrain its rig path stands over, for an actor that does not land.
    terrain: Option<Arc<Terrain>>,
}

#[derive(Clone, Default)]
pub struct AircraftPlan {
    flights: Vec<Flight>,
    /// Each flight's and each launch's animation seed, drawn from its own identity
    /// ([`AircraftPlan::assign_seeds`]).
    flight_seeds: Vec<u32>,
    launch_seeds: Vec<u32>,
    /// Each placed scenario's run on the viewer's frames.
    runs: Vec<Run>,
    /// The presentation time the runs last advanced to.
    ran_at: Option<f32>,
    /// Missile scenarios that finished on a frame since they were taken.
    finished: Vec<super::strike_effects::Impact>,
    /// Each flight's drawn transform as the last frame eased it, by its
    /// identity, so a later hour's plan of the same flights carries it on.
    eased: BTreeMap<FlightKey, Eased>,
    /// The presentation time the drawn transforms last eased to.
    eased_at: Option<f32>,
    launches: Vec<launches::Launch>,
    /// Whether explosions were taken past the launch clock's opening instant.
    opened: bool,
    /// The runs it started anew at its install.
    restarts: usize,
    /// While the flight record is kept: the flights drawn the frame before, by identity, and those
    /// of them this frame stood on a fresh transform instead of easing on ([`Self::field`]).
    drawn_keys: BTreeSet<FlightKey>,
    snapped: usize,
    pub diagnostics: Vec<String>,
    /// What its build took, by part.
    pub marks: Marks,
}

/// One build's parts, in nanoseconds of the building thread: the sources read (links,
/// participants, registry, sortie sources, target scans), the wings admitted in visit order, the
/// scenarios generated with their launch records, the training patrols, and the flights' animation
/// requests, each without the model selection by the content's scripts and the placement (routes,
/// terrain) made inside them, which are counted apart; and the wings admitted and scenarios placed.
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
    /// Scenarios and their flights placed anew, and kept from the build before ([`Kept`]).
    pub scenarios: usize,
    pub flights_placed: usize,
    pub kept: usize,
    pub flights_kept: usize,
}

/// What a worker keeps from one build to the next, for one maker's assets and map: each scenario's
/// placed flights (routes, terrain and animation requests) by everything placing them reads, so a
/// scenario whose wings keep their mission, region, target province, take-off point, model and
/// draws is not placed again. It holds only what the last build placed or kept.
#[derive(Default)]
pub struct Kept {
    placed: HashMap<Placing, Vec<Flight>>,
    /// The regions' states, what the Provinces rows they were read from read (every province's
    /// state, in row order) and the view those came from: the strategic and scout scans read the
    /// states of every region of every build, and nothing but those two columns decides them
    /// ([`RegionStates`]).
    regions: Option<(ViewIdentity, Arc<Vec<u64>>, Arc<RegionStates>)>,
}

impl Kept {
    /// The regions' states of `frame`, read again only when the provinces' own rows move: a view
    /// made anew in another column reads the same.
    fn regions(&mut self, frame: &DecodedFrame, region_provinces: &BTreeMap<u32, Vec<u32>>) -> io::Result<Arc<RegionStates>> {
        let view = &frame.tables[replay_data::rows::TableId::Provinces.index()];
        if let Some((identity, _, held)) = &self.regions
            && identity.is(view)
        {
            viewer_foundation::view_record::add("air scans", "regions kept", 1);
            return Ok(Arc::clone(held));
        }
        let read = Arc::new(RegionStates::states_read(frame)?);
        if let Some((identity, before, held)) = &mut self.regions
            && *before == read
        {
            viewer_foundation::view_record::add("air scans", "regions stand", 1);
            let held = Arc::clone(held);
            (*identity, *before) = (view.identity(), read);
            return Ok(held);
        }
        viewer_foundation::view_record::add("air scans", "regions read", 1);
        let states = Arc::new(RegionStates::read(frame, region_provinces)?);
        self.regions = Some((view.identity(), read, Arc::clone(&states)));
        Ok(states)
    }
}

/// Everything a scenario's placement reads that can differ from build to build; the rest (the
/// rig named `rig`, the province's centroid and the heights) is the maker's own content.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Placing {
    rig: String,
    mission: AirMission,
    region: u32,
    province: u32,
    start: [u32; 3],
    yaw: u32,
    phase: u32,
    /// Each side's actor: its mesh, wing, take-off point and whether it plays `explode`.
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

/// One build's keeping: what the build before placed, what this one placed or kept, and the
/// flights it placed anew, whose requests are made at the end.
#[derive(Default)]
struct Keeping {
    before: HashMap<Placing, Vec<Flight>>,
    now: HashMap<Placing, Vec<Flight>>,
    fresh: Vec<(Placing, Range<usize>)>,
}

impl Keeping {
    /// The flights placed for `key` by the build before, or earlier in this one.
    fn take(&mut self, key: &Placing) -> Option<Vec<Flight>> {
        self.before.remove(key).or_else(|| self.now.get(key).cloned())
    }
}

impl Marks {
    /// Each timed part by name, in the order a build runs them.
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

/// Launch explosions' impact ids set this bit, apart from the flights'.
const LAUNCH_IMPACT: u64 = 1 << 63;

/// What starting a launch record reads.
struct LaunchSources<'a, 'c> {
    frame: &'a DecodedFrame,
    assets: &'a AircraftAssets,
    carriers: &'a CarrierSources<'c>,
    impact_place: &'a dyn Fn(u32) -> Option<([f32; 3], [f32; 4])>,
    clock_speed: f32,
}

/// One pass's launch records over every region, in visit order: a base's
/// province holds one live record ([`launches`]).
#[derive(Default)]
struct LaunchRecords {
    bases: BTreeSet<u32>,
    /// Requests that start no record, and records that are not drawn, by
    /// reason.
    unstarted: BTreeMap<String, usize>,
    undrawn: BTreeMap<String, usize>,
    /// Each mission region's explosion places, shared by its launches.
    places: BTreeMap<u32, launches::ExplosionPlaces>,
    /// Launches drawn so far: the next one's index in the plan.
    started: usize,
}

/// A wing's launch entity: its primary equipment type's token spelling with
/// `_entity`, as the game builds the name.
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
    /// Starts `request`'s record unless its base holds one, and returns the
    /// launch to draw. A name no entity declares starts no record; a record
    /// the viewer cannot draw still holds its base.
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
        // The index the plan will give it, which seeds its player.
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
        // The entity plays its default state until its own player reads
        // its root clip's length.
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

/// The game skips a listed ID the province
/// table does not hold and counts every other listing, duplicates included,
/// in the region's total and its land (kind bit 0) or sea (bits 0 and 1
/// clear) count. The skips are counted in one warning.
/// A_/d_ choose the attacking/defending array; without a
/// prefix both arrays receive the callback. A digit chooses a one-based
/// member; zero or an out-of-range number broadcasts. Native skips exactly
/// two bytes after a numeric prefix, even for a multi-digit number.
fn flight_requests(
    flight: &Flight,
    rig: &Rig,
    actor: &crate::map::entity_animation::Rig,
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
    warnings: &mut Vec<installed_content_core::Diagnostic>,
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
        warnings.push((installed_content_core::DiagnosticKind::Warning, format!(
            "strategic regions list {skipped} provinces missing from the map definitions; the game skips them"
        )));
    }
    (members, fractions)
}

use viewer_foundation::map::air_reachability::Scans;

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

/// Where a wing's scenarios take off and its launch effects stand, by the
/// game's base position finder for a state base
/// ([`launches::Content::site`]), or its carrier.
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

/// The map a scenario is placed on.
struct Placement<'a, 'h> {
    geometry: &'a ProvinceGeometry,
    heights: &'a HeightField<'h>,
    defines: &'a FlightDefines,
}

/// An actor entity's rig and default joint palette.
type ActorSource = (Arc<crate::map::entity_animation::Rig>, Vec<PaletteJoint>);

/// What every scenario of one wing's batch shares.
struct Batch<'a> {
    wing: u64,
    region: u32,
    mission: AirMission,
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
    /// Whether its entity plays `explode` in the near-ground band.
    explode: Explode,
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

/// SplitMix64's finaliser over `value` into `state`: the stable mix the animation seeds are drawn
/// with, so a seed is the same in every build of a plan and on every machine.
fn mix(state: u64, value: u64) -> u64 {
    let mut z = state ^ value.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// `hash`'s seed, or the next seed derived from it that nothing in the plan has taken.
fn claim(taken: &mut BTreeSet<u32>, mut hash: u64) -> u32 {
    loop {
        let seed = hash as u32;
        if taken.insert(seed) {
            return seed;
        }
        hash = mix(hash, 1);
    }
}

/// A flight's identity ([`FlightKey`]) mixed: its wing, region, rig, track, anchor and heading.
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

/// A launch's identity mixed: everything [`launches::Launch::same_source`] reads of its source.
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
    /// One scenario of `batch` at `province`: the game anchors it at the
    /// province's integer pixel mean.
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
        // The game's clamped bilinear terrain sample at the integer centroid.
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
            // The game attempts both sides independently and synchronizes
            // only actors successfully appended to the scenario.
            let Some(actor) = (if rig.sides[track] == 0 {
                batch.attacker.as_ref()
            } else {
                batch.defender.as_ref()
            }) else {
                continue;
            };
            let lands = rig.lands[track];
            // The pixels its drawn rig path can stand over.
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
            // An unbound actor's analysed poses are zero: it starts and ends
            // at the rig origin without a heading.
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

    /// `ours` is the viewing country's canonical slot: admission reads nothing else of what the
    /// viewer sees, whatever the fog-of-war switch.
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

    /// [`Self::build`], keeping from `kept` each scenario the build before placed whose placement
    /// reads the same, and leaving in it what this build placed or kept.
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
        // Shared by every scan: one war census and one family set per hour.
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
        let scan_lap = |name: &'static str, started: Instant| viewer_foundation::view_record::add("air scans", name, since(started));
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
                // The scenario's orientation reference remains available
                // even when the designated attacking entity is refused.
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

    /// Bytes the plan holds: flights with their routes, requests and terrain, runs and launches.
    pub fn bytes(&self) -> usize {
        use std::mem::size_of;
        let flights: usize = self
            .flights
            .iter()
            .map(|flight| {
                size_of::<Flight>()
                    + (flight.outgoing.points.capacity() + flight.incoming.points.capacity()) * 16
                    + flight.rig.capacity()
                    + flight.requests.len() * size_of::<crate::map::entity_animation::Request>()
                    + flight.terrain.as_ref().map_or(0, |terrain| terrain.values.capacity())
            })
            .sum();
        flights
            + self.runs.iter().map(|run| size_of::<Run>() + run.grounds.capacity()).sum::<usize>()
            + self.launches.capacity() * size_of::<launches::Launch>()
            + self.diagnostics.iter().map(String::capacity).sum::<usize>()
    }

    /// Flights, launches and diagnostics the plan holds.
    pub fn counts(&self) -> [usize; 3] {
        [self.flights.len(), self.launches.len(), self.diagnostics.len()]
    }

    /// Entries of two plans built for an hour compared, entries differing, and the first
    /// described: every flight as placed, every run, launch and diagnostic in order. What the
    /// frames advanced since (eased transforms, run clocks) is not compared.
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

    /// The missile explosions of scenarios that finished on a frame since
    /// the last call, each as its scenario's cycle ended ([`Self::field`]),
    /// and of launch-flagged launch records whose entity finishes after
    /// presentation time `from` and by `to`. The first call that reaches
    /// the launch clock's opening instant from at or before it also takes
    /// the records that finish at that instant, which no `(from, to]`
    /// covers; plans carry that across swaps ([`Self::carry_opening`]).
    pub fn missile_explosions(&mut self, from: f32, to: f32) -> Vec<super::strike_effects::Impact> {
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
        // Every later finish is after the opening instant.
        let from = if opening { from.max(0.0) } else { from };
        if to > from {
            out.extend(self.launch_explosions(|launch| launch.finished(from, to)));
        }
        out
    }

    /// Keeps `previous`'s record of having passed the launch clock's
    /// opening instant, so a plan swapped in later never takes it again.
    pub fn carry_opening(&mut self, previous: &Self) {
        self.opened |= previous.opened;
    }

    /// Advances every scenario's run to presentation time `seconds`, as the
    /// game's update does each frame. A run starts, and a clock that goes
    /// back restarts it, at its phase. On a frame inside the rig each actor
    /// without a landing route meets the near-ground band on the pose it
    /// draws then and the terrain under it: the rocket is done at or below
    /// the terrain; at or below the terrain plus the band's offset an actor
    /// requests `explode` and stays there, or without that state is hidden;
    /// an exploding actor is done, and hidden for good, once its own player
    /// (`played`, the previous frame's) reads the band's delay; a hidden one
    /// is done while it is at or below the band. The cycle ends on a frame
    /// where every actor of a scenario none of whose actors lands is done;
    /// a missile scenario's ending cycle plays its explosion.
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
            let finish = |run: &Run, finished: &mut Vec<super::strike_effects::Impact>| {
                for index in run.flights.clone() {
                    if let Some((province, position, bounds)) = self.flights[index].impact {
                        finished.push(super::strike_effects::Impact {
                            id: u64::from(
                                self.flight_seeds
                                    .get(index)
                                    .copied()
                                    .unwrap_or(index as u32),
                            ) << 32
                                | u64::from(run.cycle),
                            province,
                            effect: super::strike_effects::Effect::MissileExplosion,
                            position,
                            bounds,
                        });
                    }
                }
            };
            let mut time = elapsed - run.start;
            if time >= total {
                // Only the last finished cycle plays: its province holds one effect.
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
                        // Once it has read the delay it is hidden and stays done.
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

    /// The missile explosions of launch-flagged launch records whose entity
    /// finishes after `from` and by `to`, on the presentation clock: the
    /// last record of each launch, keyed apart from the flights'.
    fn launch_explosions<'a>(
        &'a self,
        finished: impl Fn(&launches::Launch) -> Option<u32> + 'a,
    ) -> impl Iterator<Item = super::strike_effects::Impact> + 'a {
        self.launches
            .iter()
            .enumerate()
            .filter_map(move |(index, launch)| {
                launch.explosions.as_ref()?;
                let record = finished(launch)?;
                let (province, position, bounds) = launch.explosion(record)?;
                Some(super::strike_effects::Impact {
                    id: LAUNCH_IMPACT
                        | u64::from(
                            self.launch_seed(index)
                                ^ launches::Launch::instance_for(0).animation_seed,
                        ) << 32
                        | u64::from(record),
                    province,
                    effect: super::strike_effects::Effect::MissileExplosion,
                    position,
                    bounds,
                })
            })
    }

    /// A plan of rootless rockets, one per `(province, anchor)`, each a
    /// two-second take-off ending its scenario as its rig starts, run on a
    /// clock of speed one at the frames `frames`, and the explosions those
    /// frames finish.
    #[cfg(test)]
    pub(crate) fn rocket_explosions_for_tests(
        provinces: &[(u32, [f32; 3])],
        frames: &[f32],
    ) -> Vec<Vec<super::strike_effects::Impact>> {
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
        let defines = FlightDefines::for_tests();
        frames
            .iter()
            .map(|&frame| {
                plan.advance(
                    &rigs,
                    &defines,
                    Some(Band {
                        offset: 0.0,
                        explosion_delay: 0.6,
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

    /// The flights at `seconds` as their runs left them, each drawn where the
    /// game's per-frame easing has brought it ([`Eased`]): the frame's time
    /// since the last counts 30 Hz frames, and the turn steps by the aircraft
    /// clock's speed too. An actor that starts a new cycle starts on its
    /// take-off route's first point, as the game creates it there; one first
    /// seen mid-flight, or after the clock went back, stands on its pose. An
    /// exploding actor keeps the transform it had, as the game stops easing
    /// it. The band reads the poses, not the eased transforms.
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
        // The game eases an actor on the frame it enters the band, then no more.
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

    /// Every flight's and launch's animation seed, each drawn from its own identity alone, and
    /// each launch's clip length from its seed. A plan built for an hour then holds the same
    /// seeds whichever plans came before it, so a flight that goes on across an install keeps its
    /// player, and a viewer that played to the hour, one that sought to it and one that opened at
    /// it draw the same aircraft. Two entries whose identity hashes alike take the next derived
    /// seed, so no two share a player.
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
        continuity: viewer_foundation::map::presentation::Continuity,
    ) -> io::Result<()> {
        if continuity == viewer_foundation::map::presentation::Continuity::Reset {
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
        // Each run's run of the plan before: the same flights.
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

    /// The runs its install started anew: new players, nothing carried.
    pub fn restarts(&self) -> usize {
        self.restarts
    }

    /// Takes over the drawn transforms of the plan this one replaces, for the
    /// flights the two share.
    pub fn carry_easing(&mut self, previous: &mut Self) {
        self.drawn_keys = std::mem::take(&mut previous.drawn_keys);
        let keys: BTreeSet<FlightKey> = self.flights.iter().map(Flight::key).collect();
        self.eased = std::mem::take(&mut previous.eased);
        self.eased.retain(|key, _| keys.contains(key));
        self.eased_at = previous.eased_at;
    }

    /// The flights at `seconds` as their runs left them ([`Self::advance`]).
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

    /// The launch entities shown at presentation time `seconds`.
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

    /// The aircraft in the air at `seconds`, every mission placed so each keeps its player,
    /// clock and events, and the animation keys of those whose mission is not `shown`, which
    /// stay placed but draw nothing ([`crate::map::models::ModelPass::set_hidden`]). Launch
    /// effects are missiles.
    ///
    /// Each scenario's run advances to `seconds` first ([`Self::advance`]),
    /// reading the actors' players as the previous frame left them.
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
        if viewer_foundation::view_record::on() {
            // `jumps`: a flight drawn the frame before that stands on a fresh transform instead of
            // easing on (its drawn transform lost at an install); a take-off at a new cycle is the
            // game's own and eases from its route's first point.
            viewer_foundation::view_record::add("flights", "jumps", self.snapped as u64);
            viewer_foundation::view_record::set("flights", "running", self.runs.len() as u64);
            self.drawn_keys = placed.iter().map(|&(index, _)| self.flights[index].key()).collect();
        }
        // An actor exploding in the band asks for `explode` at its entry.
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
        // Actors the band hid stay placed, players stepping, and draw nothing.
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
        // A launch entity plays its default state from its spawn.
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

    // Asset diagnostics can grow while another plan bakes; these lines belong only to this plan.
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
