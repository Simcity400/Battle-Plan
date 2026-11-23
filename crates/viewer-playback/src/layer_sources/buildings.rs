use viewer_layers_infrastructure::buildings::Catalogue;
use crate::document::Content;
use viewer_buildings::map_buildings::{CityTerrain, MapBuildingAssets};

impl Catalogue for Content {
    fn assets(&self) -> &MapBuildingAssets {
        &self.map_buildings
    }

    fn city(&self) -> CityTerrain<'_> {
        CityTerrain { width: self.map.width, height: self.map.height, spawn_city: self.assets.spawn_city.as_slice() }
    }

    fn sprawl_cutoff(&self) -> f32 {
        self.map_defines.sprawl_cutoff
    }
}
