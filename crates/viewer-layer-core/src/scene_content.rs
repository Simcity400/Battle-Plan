use std::{collections::BTreeMap, io, sync::Arc};
use replay_frame::frame::DecodedFrame;
use viewer_counters::map::counters;
use viewer_gpu::{layers::Omissions, province_map::HeightField};
use viewer_unit_models::trade_routes::{self, ConvoyLine, TradeRouteDefines, TradeRoutes};

pub mod trade_route_read;
use trade_route_read::TradeRouteRead;


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayLayers {
    pub pins: bool,
    pub bases: bool,
    pub raids: bool,
    pub raids_hovered: bool,
    pub labels: bool,
    pub counters: counters::CounterDisplay,
    pub land_combats: bool,
    pub naval_markers: bool,
}

impl Default for OverlayLayers {
    fn default() -> Self {
        Self {
            pins: true,
            bases: true,
            raids: true,
            raids_hovered: true,
            labels: true,
            counters: counters::CounterDisplay::default(),
            land_combats: true,
            naval_markers: true,
        }
    }
}

impl OverlayLayers {
    pub fn shown(
        graphics: &dashboard_core::protocol::MapGraphics,
        owners: Option<Arc<std::collections::BTreeSet<u32>>>,
    ) -> Self {
        let part = |shown| graphics.counter(shown);
        Self {
            pins: part(graphics.theatre_pins),
            bases: part(graphics.base_icons),
            raids: part(graphics.raid_icons),
            raids_hovered: true,
            labels: graphics.city_names,
            counters: counters::CounterDisplay {
                land: part(graphics.land_counters),
                railway_guns: part(graphics.railway_gun_counters),
                naval: part(graphics.naval_counters),
                ports: part(graphics.port_counters),
                owners,
            },
            land_combats: part(graphics.land_combat_markers),
            naval_markers: part(graphics.naval_markers),
        }
    }

    pub fn omitting(omitted: Omissions) -> Self {
        let counters = !omitted.counters;
        let labels = !omitted.labels;
        Self {
            pins: counters,
            bases: counters,
            raids: labels,
            raids_hovered: labels,
            labels,
            counters: counters::CounterDisplay {
                land: counters,
                railway_guns: counters,
                naval: counters,
                ports: counters,
                owners: None,
            },
            land_combats: counters,
            naval_markers: counters,
        }
    }
}

pub struct KeptConvoys {
    read: TradeRouteRead,
    ours: u32,
    routes: Arc<TradeRoutes>,
    convoys: Arc<Vec<ConvoyLine>>,
}

impl KeptConvoys {
    pub fn for_hour(
        frame: &DecodedFrame,
        ours: u32,
        kept: &mut Option<Self>,
        defines: &Result<TradeRouteDefines, String>,
        lines: impl FnOnce() -> Arc<TradeRoutes>,
    ) -> Arc<Vec<ConvoyLine>> {
        use replay_data::rows::TableId;
        let read = TradeRouteRead::of(frame, kept.as_ref().map(|held| &held.read));
        if let Some(held) = kept.as_mut()
            && held.ours == ours
            && held.read.same(&read)
        {
            if held.read.assigned_same(&read) {
                held.read = read;
                viewer_runtime::view_record::add("trade routes", "convoys kept", 1);
                return Arc::clone(&held.convoys);
            }
            if let Ok(defines) = defines
                && let Some(made) = trade_routes::sailed(&held.routes, &defines.convoys, read.assigned_of(TableId::ResourceOrigins), read.assigned_of(TableId::ConvoyClients))
            {
                let made = Arc::new(made);
                static KEPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                if viewer_runtime::install_check::due(&KEPT) {
                    let whole = lines();
                    if whole.convoys != *made {
                        viewer_runtime::install_check::found("trade route convoys", format!("hour {}: {} convoy lines kept, {} built whole", frame.hour, made.len(), whole.convoys.len()));
                    }
                }
                (held.read, held.convoys) = (read, Arc::clone(&made));
                viewer_runtime::view_record::add("trade routes", "convoys made again", 1);
                return made;
            }
        }
        viewer_runtime::view_record::add("trade routes", "lines built whole", 1);
        let routes = lines();
        let convoys = Arc::new(routes.convoys.clone());
        *kept = Some(Self { read, ours, routes, convoys: Arc::clone(&convoys) });
        convoys
    }
}

pub fn links_of(frame: &DecodedFrame) -> io::Result<BTreeMap<u32, u32>> {
    Ok(viewer_foundation::tables::core::<replay_data::rows::tables::CountryLinks>(frame)?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect())
}

pub struct SceneContent {
    pub tables: Option<engine_executable::executable_tables::Tables>,
    pub products: map_products::ProductContent,
    pub map: Arc<content_reader_core::map::MapData>,
    pub map_defines: viewer_gpu::camera::MapDefines,
    pub assets: viewer_world::scene_content::MapAssets,
    pub border_meshes: Arc<viewer_borders::borders::BorderMeshes>,
    pub counters: Arc<viewer_counters::map::counters::CounterContent>,
    pub radar_centers: viewer_aircraft::radar_symbols::Centers,
    pub map_buildings: viewer_buildings::map_buildings::MapBuildingAssets,
    pub rail_assets: viewer_rail::railways::RailAssets,
    pub aircraft: Arc<viewer_aircraft::aircraft::AircraftAssets>,
    pub trains: Arc<viewer_rail::trains::TrainAssets>,
    pub bridges: viewer_rail::railways::Bridges,
    pub unit_models: viewer_unit_models::unit_models::UnitModelAssets,
    pub weather_effects: Arc<viewer_weather_effects::weather_effects::Assets>,
    pub movement: Option<Arc<viewer_foundation::movement_timing::MovementContent>>,
    pub province_geometry: Arc<viewer_gpu::province_geometry::ProvinceGeometry>,
}

impl SceneContent {
    pub fn heights(&self) -> HeightField<'_> {
        HeightField {
            width: self.assets.heightmap.width,
            height: self.assets.heightmap.height,
            values: self.assets.heightmap_texels(),
        }
    }
}
