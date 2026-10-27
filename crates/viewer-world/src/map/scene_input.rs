//! Scene inputs preserve setter order, returned products and refusal boundaries.

use super::*;
use crate::map::{
    aircraft::{AircraftAssets, AircraftPlan},
    animated_map_text::events,
    railways::PreparedRailGeometry,
    trains::{TrainAssets, TrainPlan},
};
use viewer_foundation::map::{native_resources::GuiResources, province_map::HeightField};

/// Unboxed hour products move directly into their setters without another allocation.
#[allow(clippy::large_enum_variant)]
pub enum SceneInput<'a> {
    WarmTrains(&'a Arc<TrainAssets>),
    WarmUnitModels(&'a UnitModelAssets),
    WarmAircraft(&'a Arc<AircraftAssets>),
    WarmMapBuildings(&'a MapBuildingAssets),
    WarmBridges(&'a Bridges),
    WarmRailways(&'a RailAssets),
    WarmOrders(&'a OrderAssets),
    WarmWeather(Arc<crate::map::weather_effects::Assets>),
    WarmPolitical {
        map: &'a installed_content_core::map::MapData,
        strategic_regions: &'a BTreeMap<u32, installed_content_regions::regions::StrategicRegion>,
        field: &'a viewer_foundation::map::gradient_math::FieldDefines,
        cache: &'a crate::map::political_gpu::Cache,
    },
    SetBorders {
        assets: &'a BorderAssets,
        meshes: &'a BorderMeshes,
        straits: &'a [StraitVertex],
    },
    SetRivers {
        assets: &'a RiverAssets,
        mesh: &'a RiverMesh,
    },
    SetTrees {
        assets: &'a TreeAssets,
        geometry: TreeGeometry,
        field: &'a TreeField,
    },
    SetSprawl(&'a Sprawl),
    SetMapFrame(&'a FrameAssets),
    InstallAnimatedText {
        assets: &'a crate::map::animated_map_text::Assets,
        gui: &'a GuiResources,
    },
    SetPreparedRailways {
        assets: &'a RailAssets,
        geometry: Arc<PreparedRailGeometry>,
    },
    SetBridges {
        bridges: &'a Bridges,
        field: ModelField,
    },
    SetMapBuildings {
        assets: &'a MapBuildingAssets,
        prepared: map_display_data::buildings::Buildings,
        passed: Option<&'a [&'a map_display_data::buildings::Buildings]>,
    },
    SetTrains {
        assets: Arc<TrainAssets>,
        plan: TrainPlan,
        continuity: Continuity,
    },
    SetAircraft {
        assets: Arc<AircraftAssets>,
        plan: AircraftPlan,
    },
    RestartUnitModels(Option<&'a BTreeSet<u32>>),
    SetOrders {
        assets: &'a OrderAssets,
        field: &'a OrderField,
    },
    SetPreparedWeatherEffects {
        assets: Arc<crate::map::weather_effects::Assets>,
        prepared: crate::map::weather_effects::Prepared,
        continuity: Continuity,
    },
    SetPolitical(&'a Arc<Regions>),
    SetBorderPlans {
        classes: &'a crate::map::border_pass::ClassPlan,
        zone: &'a crate::map::demilitarized::ZonePlan,
    },
    SetSnowMud {
        snow: &'a Arc<crate::map::weather::SnowMud>,
        continuity: Continuity,
    },
    SetFogOfWar {
        texels: &'a map_display_data::Paged<u8>,
        fog: bool,
    },
    PresentHours {
        at: (replay_data::BranchId, i32),
        seek: bool,
        hours: Vec<(
            &'a crate::map::strike_effects::Impacts,
            &'a events::Prepared,
        )>,
        continuity: Continuity,
        anchor: &'a dyn Fn(u32) -> Option<[f32; 3]>,
        heights: HeightField<'a>,
        gui: &'a GuiResources,
    },
    MissingMapTextEvents {
        at: (replay_data::BranchId, i32),
        seek: bool,
        reason: String,
    },
    SetUnitModels {
        assets: &'a UnitModelAssets,
        field: crate::map::unit_history::Field,
        steady: bool,
    },
    UseWorkers(viewer_foundation::parallel::Workers),
    ShareProvinceSchedule(&'a Shared),
    AdvanceProvinces {
        camera: Camera,
        size: [u32; 2],
    },
    /// The selected units' movement arrows (`movement_arrows.set`).
    SetMovementArrows {
        epoch: (u64, i32, u32),
        routes: Vec<viewer_foundation::map::movement_routes::Route>,
        warnings: Vec<String>,
        heights: HeightField<'a>,
        width: f32,
        selected: &'a BTreeSet<viewer_foundation::unit_handle::Unit>,
    },
    /// Arrow admission geometry (`unit_history.prepare_arrow_bounds`).
    PrepareArrowBounds {
        map: &'a installed_content_core::map::MapData,
        known: &'a Arc<BTreeMap<u32, [f32; 4]>>,
    },
    /// The units' selection (`unit_history.set_selection`).
    SetUnitSelection(&'a BTreeSet<viewer_foundation::unit_handle::Unit>),
    /// The selected units' provinces (`unit_history.select_provinces`).
    SelectProvinces(BTreeSet<u32>),
    SetRaidUnits(Vec<(usize, crate::map::models::ModelInstance)>),
    SetFactionHighlight {
        plan: &'a FactionHighlight,
        strategic_regions: &'a [u32],
        color: [f32; 4],
    },
    SetAirRanges(&'a crate::map::radar_symbols::Field),
    SetAirTransfers(&'a crate::map::radar_symbols::Field),
    SetRadar {
        field: &'a crate::map::radar_symbols::Field,
        shown: bool,
        hovered: Option<(u32, u32)>,
    },
    SetAirAssignment(&'a crate::map::air_assignment::Field),
    SetStrategicStyles(&'a BTreeMap<u32, crate::map::map_mode::Style>),
    SetGraphics(dashboard_core::protocol::MapGraphics),
    SetPlayer(Option<PlayerModel<'a>>),
    SetVisibility(viewer_foundation::map::mode_visibility::Visibility),
    LabelFrame(u64),
    CutProvinceFrame(f32),
}

/// What an applied input returns to its host: the replaced product a setter hands back, if any.
#[allow(clippy::large_enum_variant)]
pub enum SceneOutput {
    Applied,
    PreparedRailways(Arc<PreparedRailGeometry>),
    Trains(Option<TrainPlan>),
    Aircraft(Option<AircraftPlan>),
    WeatherEffects(crate::map::weather_effects::Retired),
}

impl SceneOutput {
    /// What [`SceneInput::SetPreparedRailways`] replaced.
    pub fn into_prepared_railways(self) -> Arc<PreparedRailGeometry> {
        match self {
            Self::PreparedRailways(geometry) => geometry,
            _ => unreachable!("only SetPreparedRailways returns railway geometry"),
        }
    }

    /// What [`SceneInput::SetTrains`] replaced.
    pub fn into_trains(self) -> Option<TrainPlan> {
        match self {
            Self::Trains(plan) => plan,
            _ => unreachable!("only SetTrains returns a train plan"),
        }
    }

    /// What [`SceneInput::SetAircraft`] replaced.
    pub fn into_aircraft(self) -> Option<AircraftPlan> {
        match self {
            Self::Aircraft(plan) => plan,
            _ => unreachable!("only SetAircraft returns an aircraft plan"),
        }
    }

    /// What [`SceneInput::SetPreparedWeatherEffects`] retired.
    pub fn into_weather_effects(self) -> crate::map::weather_effects::Retired {
        match self {
            Self::WeatherEffects(retired) => retired,
            _ => unreachable!("only SetPreparedWeatherEffects returns retired effects"),
        }
    }
}

impl Scene {
    /// Returns the original setter's replaced product or refusal unchanged.
    pub fn apply(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        input: SceneInput<'_>,
    ) -> io::Result<SceneOutput> {
        let _cost = replay_frame::cpu::scope("viewer", Some("upload"));
        let applied = SceneOutput::Applied;
        Ok(match input {
            SceneInput::WarmTrains(assets) => {
                self.warm_trains(device, queue, assets)?;
                applied
            }
            SceneInput::WarmUnitModels(assets) => {
                self.warm_unit_models(device, queue, assets)?;
                applied
            }
            SceneInput::WarmAircraft(assets) => {
                self.warm_aircraft(device, queue, assets)?;
                applied
            }
            SceneInput::WarmMapBuildings(assets) => {
                self.warm_map_buildings(device, queue, assets)?;
                applied
            }
            SceneInput::WarmBridges(bridges) => {
                self.warm_bridges(device, queue, bridges)?;
                applied
            }
            SceneInput::WarmRailways(assets) => {
                self.warm_railways(device, queue, assets)?;
                applied
            }
            SceneInput::WarmOrders(assets) => {
                self.warm_orders(device, queue, assets)?;
                applied
            }
            SceneInput::WarmWeather(assets) => {
                self.warm_weather(device, queue, assets)?;
                applied
            }
            SceneInput::WarmPolitical {
                map,
                strategic_regions,
                field,
                cache,
            } => {
                self.warm_political(device, queue, map, strategic_regions, field, cache)?;
                applied
            }
            SceneInput::SetBorders {
                assets,
                meshes,
                straits,
            } => {
                self.set_borders(device, queue, assets, meshes, straits)?;
                applied
            }
            SceneInput::SetRivers { assets, mesh } => {
                self.set_rivers(device, queue, assets, mesh)?;
                applied
            }
            SceneInput::SetTrees {
                assets,
                geometry,
                field,
            } => {
                self.set_trees(device, queue, assets, geometry, field)?;
                applied
            }
            SceneInput::SetSprawl(sprawl) => {
                self.set_sprawl(device, queue, sprawl)?;
                applied
            }
            SceneInput::SetMapFrame(assets) => {
                self.set_map_frame(device, queue, assets)?;
                applied
            }
            SceneInput::InstallAnimatedText { assets, gui } => {
                self.install_animated_text(assets, gui);
                applied
            }
            SceneInput::SetPreparedRailways { assets, geometry } => SceneOutput::PreparedRailways(
                self.set_prepared_railways(device, queue, assets, geometry)?,
            ),
            SceneInput::SetBridges { bridges, field } => {
                self.set_bridges(device, queue, bridges, field)?;
                applied
            }
            SceneInput::SetMapBuildings {
                assets,
                prepared,
                passed,
            } => {
                self.set_map_buildings(device, queue, assets, prepared, passed)?;
                applied
            }
            SceneInput::SetTrains {
                assets,
                plan,
                continuity,
            } => SceneOutput::Trains(self.set_trains(assets, plan, continuity)?),
            SceneInput::SetAircraft { assets, plan } => {
                SceneOutput::Aircraft(self.set_aircraft(assets, plan)?)
            }
            SceneInput::RestartUnitModels(provinces) => {
                self.restart_unit_models(provinces);
                applied
            }
            SceneInput::SetOrders { assets, field } => {
                self.set_orders(device, queue, assets, field)?;
                applied
            }
            SceneInput::SetPreparedWeatherEffects {
                assets,
                prepared,
                continuity,
            } => SceneOutput::WeatherEffects(
                self.set_prepared_weather_effects(device, queue, assets, prepared, continuity)?,
            ),
            SceneInput::SetPolitical(regions) => {
                self.set_political(device, queue, regions)?;
                applied
            }
            SceneInput::SetBorderPlans { classes, zone } => {
                self.set_border_plans(device, classes, zone)?;
                applied
            }
            SceneInput::SetSnowMud { snow, continuity } => {
                self.set_snow_mud(device, queue, snow, continuity)?;
                applied
            }
            SceneInput::SetFogOfWar { texels, fog } => {
                self.set_fog_of_war(queue, texels, fog)?;
                applied
            }
            SceneInput::PresentHours {
                at,
                seek,
                hours,
                continuity,
                anchor,
                heights,
                gui,
            } => {
                self.present_hours(at, seek, hours, continuity, anchor, heights, gui);
                applied
            }
            SceneInput::MissingMapTextEvents { at, seek, reason } => {
                self.missing_map_text_events(at, seek, reason);
                applied
            }
            SceneInput::SetUnitModels {
                assets,
                field,
                steady,
            } => {
                self.set_unit_models(device, queue, assets, field, steady)?;
                applied
            }
            SceneInput::UseWorkers(workers) => {
                self.use_workers(workers);
                applied
            }
            SceneInput::ShareProvinceSchedule(schedule) => {
                self.share_province_schedule(schedule);
                applied
            }
            SceneInput::AdvanceProvinces { camera, size } => {
                self.advance_provinces(camera, size);
                applied
            }
            SceneInput::SetMovementArrows {
                epoch,
                routes,
                warnings,
                heights,
                width,
                selected,
            } => {
                self.movement_arrows
                    .set(epoch, routes, warnings, heights, width, selected);
                applied
            }
            SceneInput::PrepareArrowBounds { map, known } => {
                self.unit_history.prepare_arrow_bounds(map, known);
                applied
            }
            SceneInput::SetUnitSelection(selected) => {
                self.unit_history.set_selection(selected);
                applied
            }
            SceneInput::SelectProvinces(provinces) => {
                self.unit_history.select_provinces(provinces);
                applied
            }
            SceneInput::SetRaidUnits(raids) => {
                self.set_raid_units(raids);
                applied
            }
            SceneInput::SetFactionHighlight {
                plan,
                strategic_regions,
                color,
            } => {
                self.set_faction_highlight(device, plan, strategic_regions, color)?;
                applied
            }
            SceneInput::SetAirRanges(field) => {
                self.set_air_ranges(field);
                applied
            }
            SceneInput::SetAirTransfers(field) => {
                self.set_air_transfers(field);
                applied
            }
            SceneInput::SetRadar {
                field,
                shown,
                hovered,
            } => {
                self.set_radar(field, shown, hovered);
                applied
            }
            SceneInput::SetAirAssignment(field) => {
                self.set_air_assignment(queue, field)?;
                applied
            }
            SceneInput::SetStrategicStyles(styles) => {
                self.set_strategic_styles(device, queue, styles)?;
                applied
            }
            SceneInput::SetGraphics(graphics) => {
                self.set_graphics(graphics);
                applied
            }
            SceneInput::SetPlayer(player) => {
                self.set_player(device, queue, player)?;
                applied
            }
            SceneInput::SetVisibility(visibility) => {
                self.set_visibility(visibility);
                applied
            }
            SceneInput::LabelFrame(frame) => {
                self.label_frame(frame);
                applied
            }
            SceneInput::CutProvinceFrame(animation_time) => {
                self.cut_province_frame(animation_time);
                applied
            }
        })
    }
}
