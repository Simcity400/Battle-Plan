
use super::*;
use viewer_aircraft::aircraft::AircraftAssets;
use viewer_rail::trains::TrainAssets;
use viewer_native_gui::native_resources::GuiResources;
use viewer_gpu::province_map::HeightField;

#[allow(clippy::large_enum_variant, reason = "a product moves into its setter unboxed, with no second allocation")]
pub enum SceneInput<'a> {
    WarmTrains(&'a Arc<TrainAssets>),
    WarmUnitModels(&'a UnitModelAssets),
    WarmAircraft(&'a Arc<AircraftAssets>),
    WarmMapBuildings(&'a MapBuildingAssets),
    WarmBridges(&'a Bridges),
    WarmRailways(&'a RailAssets),
    WarmOrders(&'a OrderAssets),
    WarmWeather(Arc<viewer_weather_effects::weather_effects::Assets>),
    WarmPipelines,
    WarmPolitical {
        map: &'a content_reader_map::map::MapData,
        strategic_regions: &'a BTreeMap<u32, content_reader_regions::regions::StrategicRegion>,
        field: &'a viewer_gpu::gradient_math::FieldDefines,
        cache: &'a viewer_map_modes::political_gpu::Cache,
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
        assets: &'a viewer_map_text::animated_map_text::Assets,
        gui: &'a GuiResources,
    },
    SetOrders {
        assets: &'a OrderAssets,
        parts: [Option<&'a OrderField>; viewer_aircraft::order_pass::ORDER_PARTS],
    },
    SetUnitMovement {
        positions: Box<
            dyn Iterator<
                    Item = (
                        viewer_runtime::unit_handle::Unit,
                        &'a viewer_unit_reads::counter_move::Position,
                        f32,
                        f32,
                    ),
                > + 'a,
        >,
        history: &'a viewer_unit_reads::counter_move::SharedMovement,
    },
    ShareProvinceSchedule(&'a Shared),
    AdvanceProvinces {
        camera: Camera,
        size: [u32; 2],
    },
    SetMovementArrows {
        epoch: (u64, i32, u32),
        routes: Vec<viewer_unit_reads::movement_routes::Route>,
        warnings: Vec<String>,
        heights: HeightField<'a>,
        width: f32,
        selected: &'a BTreeSet<viewer_runtime::unit_handle::Unit>,
    },
    PrepareArrowBounds {
        map: &'a content_reader_map::map::MapData,
        known: &'a Arc<BTreeMap<u32, [f32; 4]>>,
    },
    SetUnitSelection(&'a BTreeSet<viewer_runtime::unit_handle::Unit>),
    SelectProvinces(BTreeSet<u32>),
    SetRaidUnits(Vec<(usize, viewer_models::ModelInstance)>),
    SetFactionHighlight {
        plan: &'a FactionHighlight,
        strategic_regions: &'a [u32],
        color: [f32; 4],
    },
    SetAirRanges(&'a viewer_aircraft::radar_symbols::Field),
    SetAirTransfers(&'a viewer_aircraft::radar_symbols::Field),
    SetRadar {
        field: &'a viewer_aircraft::radar_symbols::Field,
        shown: bool,
        hovered: Option<(u32, u32)>,
    },
    SetAirAssignment(&'a viewer_map_modes::air_assignment::Field),
    SetStrategicStyles(&'a BTreeMap<u32, map_display_data::landscape::Style>),
    SetGraphics(dashboard_core::protocol::MapGraphics),
    SetPlayer(Option<PlayerModel<'a>>),
    SetVisibility(viewer_foundation::mode_visibility::Visibility),
    LabelFrame(u64),
    CutProvinceFrame(f32),
}

impl SceneInput<'_> {
    pub fn name(&self) -> &'static str {
        match self {
            Self::WarmTrains { .. } => "WarmTrains",
            Self::WarmUnitModels { .. } => "WarmUnitModels",
            Self::WarmAircraft { .. } => "WarmAircraft",
            Self::WarmMapBuildings { .. } => "WarmMapBuildings",
            Self::WarmBridges { .. } => "WarmBridges",
            Self::WarmRailways { .. } => "WarmRailways",
            Self::WarmOrders { .. } => "WarmOrders",
            Self::WarmWeather { .. } => "WarmWeather",
            Self::WarmPolitical { .. } => "WarmPolitical",
            Self::WarmPipelines => "WarmPipelines",
            Self::SetBorders { .. } => "SetBorders",
            Self::SetRivers { .. } => "SetRivers",
            Self::SetTrees { .. } => "SetTrees",
            Self::SetSprawl { .. } => "SetSprawl",
            Self::SetMapFrame { .. } => "SetMapFrame",
            Self::InstallAnimatedText { .. } => "InstallAnimatedText",
            Self::SetOrders { .. } => "SetOrders",
            Self::SetUnitMovement { .. } => "SetUnitMovement",
            Self::ShareProvinceSchedule { .. } => "ShareProvinceSchedule",
            Self::AdvanceProvinces { .. } => "AdvanceProvinces",
            Self::SetMovementArrows { .. } => "SetMovementArrows",
            Self::PrepareArrowBounds { .. } => "PrepareArrowBounds",
            Self::SetUnitSelection { .. } => "SetUnitSelection",
            Self::SelectProvinces { .. } => "SelectProvinces",
            Self::SetRaidUnits { .. } => "SetRaidUnits",
            Self::SetFactionHighlight { .. } => "SetFactionHighlight",
            Self::SetAirRanges { .. } => "SetAirRanges",
            Self::SetAirTransfers { .. } => "SetAirTransfers",
            Self::SetRadar { .. } => "SetRadar",
            Self::SetAirAssignment { .. } => "SetAirAssignment",
            Self::SetStrategicStyles { .. } => "SetStrategicStyles",
            Self::SetGraphics { .. } => "SetGraphics",
            Self::SetPlayer { .. } => "SetPlayer",
            Self::SetVisibility { .. } => "SetVisibility",
            Self::LabelFrame { .. } => "LabelFrame",
            Self::CutProvinceFrame { .. } => "CutProvinceFrame",
        }
    }
}

impl Scene {
    pub fn apply(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        input: SceneInput<'_>,
    ) -> io::Result<()> {
        let _cost = replay_frame::cpu::scope(input.name(), Some("upload"));
        match input {
            SceneInput::WarmTrains(assets) => {
                self.warm_trains(device, queue, assets)?;
            }
            SceneInput::WarmUnitModels(assets) => {
                self.warm_unit_models(device, queue, assets)?;
            }
            SceneInput::WarmAircraft(assets) => {
                self.warm_aircraft(device, queue, assets)?;
            }
            SceneInput::WarmMapBuildings(assets) => {
                self.warm_map_buildings(device, queue, assets)?;
            }
            SceneInput::WarmBridges(bridges) => {
                self.warm_bridges(device, queue, bridges)?;
            }
            SceneInput::WarmRailways(assets) => {
                self.warm_railways(device, queue, assets)?;
            }
            SceneInput::WarmOrders(assets) => {
                self.warm_orders(device, queue, assets)?;
            }
            SceneInput::WarmWeather(assets) => {
                self.warm_weather(device, queue, assets)?;
            }
            SceneInput::WarmPipelines => {
                self.warm_pipelines(device);
            }
            SceneInput::WarmPolitical {
                map,
                strategic_regions,
                field,
                cache,
            } => {
                self.warm_political(device, queue, map, strategic_regions, field, cache)?;
            }
            SceneInput::SetBorders {
                assets,
                meshes,
                straits,
            } => {
                self.set_borders(device, queue, assets, meshes, straits)?;
            }
            SceneInput::SetRivers { assets, mesh } => {
                self.set_rivers(device, queue, assets, mesh)?;
            }
            SceneInput::SetTrees {
                assets,
                geometry,
                field,
            } => {
                self.set_trees(device, queue, assets, geometry, field)?;
            }
            SceneInput::SetSprawl(sprawl) => {
                self.set_sprawl(device, queue, sprawl)?;
            }
            SceneInput::SetMapFrame(assets) => {
                self.set_map_frame(device, queue, assets)?;
            }
            SceneInput::InstallAnimatedText { assets, gui } => {
                self.install_animated_text(assets, gui);
            }
            SceneInput::SetOrders { assets, parts } => {
                self.set_order_parts(device, queue, assets, parts)?;
            }
            SceneInput::SetUnitMovement { positions, history } => {
                self.unit_history.set_movement(positions, history);
            }
            SceneInput::ShareProvinceSchedule(schedule) => {
                self.share_province_schedule(schedule);
            }
            SceneInput::AdvanceProvinces { camera, size } => {
                self.advance_provinces(camera, size);
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
            }
            SceneInput::PrepareArrowBounds { map, known } => {
                self.unit_history.prepare_arrow_bounds(map, known);
            }
            SceneInput::SetUnitSelection(selected) => {
                self.unit_history.set_selection(selected);
            }
            SceneInput::SelectProvinces(provinces) => {
                self.unit_history.select_provinces(provinces);
            }
            SceneInput::SetRaidUnits(raids) => {
                self.set_raid_units(raids);
            }
            SceneInput::SetFactionHighlight {
                plan,
                strategic_regions,
                color,
            } => {
                self.set_faction_highlight(device, plan, strategic_regions, color)?;
            }
            SceneInput::SetAirRanges(field) => {
                self.set_air_ranges(field);
            }
            SceneInput::SetAirTransfers(field) => {
                self.set_air_transfers(field);
            }
            SceneInput::SetRadar {
                field,
                shown,
                hovered,
            } => {
                self.set_radar(field, shown, hovered);
            }
            SceneInput::SetAirAssignment(field) => {
                self.set_air_assignment(queue, field)?;
            }
            SceneInput::SetStrategicStyles(styles) => {
                self.set_strategic_styles(device, queue, styles)?;
            }
            SceneInput::SetGraphics(graphics) => {
                self.set_graphics(graphics);
            }
            SceneInput::SetPlayer(player) => {
                self.set_player(device, queue, player)?;
            }
            SceneInput::SetVisibility(visibility) => {
                self.set_visibility(visibility);
            }
            SceneInput::LabelFrame(frame) => {
                self.label_frame(frame);
            }
            SceneInput::CutProvinceFrame(animation_time) => {
                self.cut_province_frame(animation_time);
            }
        }
        Ok(())
    }
}
