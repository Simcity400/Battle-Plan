//! The lines a map's layers give for the warnings its window shows.

/// The political fill's limitation lines: the political, naval and mode fills layers' own while
/// the political layer draws (its refusal is the map's own line otherwise).
pub fn political_lines(side: &crate::playhead_host::MapSide) -> Vec<String> {
    match side.parts().filter(|_| side.draws("political")) {
        Some((_, layers)) => {
            let political = layers.iter().filter_map(|layer| layer.any().downcast_ref::<crate::layers::political::Political>()).flat_map(|layer| layer.warnings());
            let naval = layers.iter().filter_map(|layer| layer.any().downcast_ref::<crate::layers::naval::Naval>()).flat_map(|layer| layer.warnings());
            let modes = layers.iter().filter_map(|layer| layer.any().downcast_ref::<crate::layers::mode_fills::ModeFills>()).flat_map(|layer| layer.lines());
            political.chain(naval).chain(modes).collect()
        }
        None => Vec::new(),
    }
}

/// The lines the playhead's map gives: its limitation lines (the tables' notes, the drawn layers'
/// own, the scripts layer's raid arrows, and each refused layer), and the battle plan's, the map
/// buildings' and the unit models' own reports, each from its layer while it draws (the ships'
/// within the units layer's while that draws).
pub fn map_lines(side: &crate::playhead_host::MapSide) -> [Vec<String>; 2] {
    let limitations = crate::playhead_host::limitations::lines(side).unwrap_or_else(|| vec![MAP_LINES_LOADING.to_owned()]);
    let mut lines = Vec::new();
    if let Some((_, layers)) = side.parts() {
        let layer = |name: &str| layers.iter().find(|layer| layer.name() == name && side.draws(name));
        if let Some(orders) = layer("orders").and_then(|layer| layer.any().downcast_ref::<crate::layers::orders::Orders>()) {
            lines.extend(orders.warnings());
        }
        if let Some(buildings) = layer("buildings").and_then(|layer| layer.any().downcast_ref::<crate::layers::buildings::BuildingsFed>()) {
            lines.extend_from_slice(buildings.warnings());
        }
        match layer("units").and_then(|layer| layer.any().downcast_ref::<crate::layers::units::Units>()) {
            Some(units) => lines.extend(units.warnings()),
            None => {
                if let Some(ships) = layer("ships").and_then(|layer| layer.any().downcast_ref::<crate::layers::ships::Ships>()) {
                    lines.extend_from_slice(ships.warnings());
                }
            }
        }
        if let Some(aircraft) = layer("aircraft").and_then(|layer| layer.any().downcast_ref::<crate::layers::aircraft::Aircraft>()) {
            lines.extend(aircraft.warning());
        }
    }
    [limitations, lines]
}

/// The map's limitation line while its head has reached no hour.
pub const MAP_LINES_LOADING: &str = "Map limitations: loading, the playback head has reached no hour yet";
