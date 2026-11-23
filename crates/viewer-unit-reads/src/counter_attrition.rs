use crate::counter_inputs::SCALE as ONE;
use std::io;

pub struct Content {
    pub training: i64,
    pub border_factor: i64,
    pub supply_threshold: i64,
    pub terrains: Vec<Terrain>,
    pub sub_units: Result<std::sync::Arc<content_reader_script::sub_units::SubUnits>, String>,
}

impl replay_derive::eta_derive::frame_terms::SubUnitContent for Content {
    fn special_forces(&self, name: &[u8]) -> Result<bool, String> {
        self.sub_units
            .as_ref()
            .map_err(Clone::clone)?
            .special_forces(name)
    }

    fn sub_unit_types(&self, name: &[u8]) -> Result<u64, String> {
        self.sub_units
            .as_ref()
            .map_err(Clone::clone)?
            .category(name)
    }
}

pub struct Terrain {
    pub name: Vec<u8>,
    pub attrition: Option<i64>,
}

impl Content {
    pub fn read(
        defines: &content_reader_defines::defines::Defines,
        terrain: &content_reader_key_names::terrain::TerrainDefinitions,
        sub_units: Result<std::sync::Arc<content_reader_script::sub_units::SubUnits>, String>,
    ) -> io::Result<Self> {
        Ok(Self {
            training: defines.fixed5("NMilitary", "TRAINING_ATTRITION")?,
            border_factor: defines.fixed5("NMilitary", "BORDER_WAR_ATTRITION_FACTOR")?,
            supply_threshold: defines
                .fixed5("NSupply", "SUPPLY_THRESHOLD_FOR_ARMY_ATTRITION")?
                .clamp(0, ONE),
            terrains: terrain
                .categories
                .iter()
                .map(|category| Terrain {
                    name: category.name.clone(),
                    attrition: category
                        .modifiers
                        .values
                        .iter()
                        .find(|value| value.name == "attrition")
                        .map(|value| value.value),
                })
                .collect(),
            sub_units,
        })
    }
}

pub struct PanelContent {
    pub terms: Content,
    pub members: std::collections::BTreeMap<u32, Vec<u32>>,
    pub neighbors: Result<std::collections::BTreeMap<u32, Vec<u32>>, String>,
    pub border: Result<viewer_foundation::border_power::Defines, String>,
}
impl PanelContent {
    pub fn read(
        layers: &content_reader_layers::ResolvedLayers,
        map: &content_reader_map::map::MapData,
        sub_units: Result<std::sync::Arc<content_reader_script::sub_units::SubUnits>, String>,
    ) -> Result<Self, String> {
        let defines = content_reader_defines::defines::read(layers).map_err(|why| why.to_string())?;
        let terrain =
            content_reader_key_names::terrain::read_definitions(layers).map_err(|why| why.to_string())?;
        let terms = Content::read(&defines, &terrain, sub_units).map_err(|why| why.to_string())?;
        let (states, _) = content_reader_states::states::read(layers, &defines, &map.definitions)
            .map_err(|why| why.to_string())?;
        let members = states
            .definitions()
            .iter()
            .map(|(&id, state)| (id, state.provinces.clone()))
            .collect();
        let neighbors = content_reader_map::province_graph::ProvinceGraph::read(layers)
            .and_then(|(graph, _)| graph.neighbor_states(&members))
            .map_err(|why| why.to_string());
        Ok(Self {
            terms,
            members,
            neighbors,
            border: viewer_foundation::border_power::Defines::load(&defines).map_err(|why| why.to_string()),
        })
    }
}
