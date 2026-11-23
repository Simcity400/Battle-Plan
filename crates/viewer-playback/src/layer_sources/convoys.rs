use viewer_layers_forces::convoys::{Route, Provinces, Read, Routes};
use std::collections::BTreeMap;
use viewer_unit_models::trade_routes::{self, Countries, Emit, ExchangeOrder, Stops, TradeRoutes};

impl Routes for crate::document::Content {
    fn stops(&self, route: &Route, reverse: bool, provinces: &Provinces) -> Result<Option<Stops>, String> {
        let (graph, geography) = self.icons.raids.mission_topology().map_err(|why| format!("the content's province graph is unavailable: {why}"))?;
        let position = |province: u32| {
            let slot = if geography.land.contains(&province) && provinces.ports.contains_key(&province) {
                viewer_foundation::unit_rules::SHIP_IN_PORT
            } else {
                viewer_foundation::unit_rules::STANDSTILL
            };
            self.counters.anchor(province, slot)
        };
        let pixels = |province| self.province_geometry.pixel_count(province);
        let crossing = |a, b| self.movement.as_ref()?.map.neighbour_crossing(a, b);
        let content = trade_routes::RouteContent {
            graph,
            geography,
            state_of: &provinces.state_of,
            ports: &provinces.ports,
            pixels: &pixels,
            crossing: &crossing,
            region_of: &self.icons.raids.regions,
            region_provinces: &self.icons.raids.region_provinces,
            state_provinces: &self.icons.raids.trigger_content.state_provinces,
            position: &position,
        };
        trade_routes::directed_stops(route, &content, reverse)
    }

    fn build(
        &self,
        ours: u32,
        tables: Read<'_>,
        countries: &Countries<'_>,
        exchanges: &BTreeMap<u32, ExchangeOrder>,
        culture: &dyn Fn(&Countries<'_>) -> Result<String, String>,
        provinces: Result<&Provinces, String>,
        stops: &mut dyn FnMut(&Route, bool) -> Result<Option<Stops>, String>,
        emit: &mut Emit<'_>,
    ) -> TradeRoutes {
        let mut out = TradeRoutes::default();
        let (graph, geography) = match self.icons.raids.mission_topology() {
            Ok(topology) => topology,
            Err(why) => {
                out.report.unavailable = Some(format!("the content's province graph is unavailable: {why}"));
                return out;
            }
        };
        let provinces = match provinces {
            Ok(provinces) => provinces,
            Err(why) => {
                out.report.unavailable = Some(why);
                return out;
            }
        };
        let position = |province: u32| {
            let slot = if geography.land.contains(&province) && provinces.ports.contains_key(&province) {
                viewer_foundation::unit_rules::SHIP_IN_PORT
            } else {
                viewer_foundation::unit_rules::STANDSTILL
            };
            self.counters.anchor(province, slot)
        };
        let pixels = |province| self.province_geometry.pixel_count(province);
        let crossing = |a, b| self.movement.as_ref()?.map.neighbour_crossing(a, b);
        let content = trade_routes::RouteContent {
            graph,
            geography,
            state_of: &provinces.state_of,
            ports: &provinces.ports,
            pixels: &pixels,
            crossing: &crossing,
            region_of: &self.icons.raids.regions,
            region_provinces: &self.icons.raids.region_provinces,
            state_provinces: &self.icons.raids.trigger_content.state_provinces,
            position: &position,
        };
        let heights = viewer_gpu::province_map::HeightField { width: self.assets.heightmap.width, height: self.assets.heightmap.height, values: self.assets.heightmap_texels() };
        let declared = |name: &str| self.unit_models.declares(name);
        let inputs = trade_routes::Inputs { orders: &self.orders, defines: self.trade_routes.as_ref().map_err(String::as_str), content: &content, heights: &heights, ours, declared: &declared };
        let checked = (|| {
            inputs.defines.map_err(|reason| format!("the trade-route defines cannot be read: {reason}"))?;
            inputs.orders.types.get(viewer_gpu::ribbon_content::TRADE_ROUTE_TYPE_NAME).ok_or_else(|| {
                format!("{} declares no arrowType {}", viewer_gpu::ribbon_content::MAPARROWS, viewer_gpu::ribbon_content::TRADE_ROUTE_TYPE_NAME)
            })?;
            let tables = tables?;
            trade_routes::build_lines(&inputs, &tables, countries, exchanges, culture, stops, emit, &mut out)
        })();
        if let Err(reason) = checked {
            out.report.unavailable = Some(reason);
        }
        out
    }
}
