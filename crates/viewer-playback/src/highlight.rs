
use viewer_layers_political::political::Political;
use map_display_data::{landscape::Regions, province_field, value_index::ValueIndex};
use std::{collections::BTreeSet, sync::Arc};
use viewer_borders::demilitarized::Zone;

#[derive(Clone)]
pub enum Over<P> {
    Layer(P),
    Mode(Arc<Regions>, map_display_data::retained::Retained<Zone>),
}

#[derive(Default)]
pub struct Picture {
    country_of: Arc<Vec<u32>>,
    owner_of: Arc<Vec<u32>>,
    state_of: Arc<Vec<u32>>,
    colours: Vec<(u32, [u8; 3])>,
    demilitarized: BTreeSet<u32>,
}

impl Picture {
    pub fn of(layer: &Political) -> Self {
        let demilitarized = layer.demilitarized();
        if layer.refused().is_some() {
            let empty = Arc::new(vec![0; layer.country_of().len()]);
            return Self { country_of: Arc::clone(&empty), owner_of: Arc::clone(&empty), state_of: empty, colours: Vec::new(), demilitarized };
        }
        Self {
            country_of: Arc::new(layer.country_of().to_vec()),
            owner_of: Arc::new(layer.owner_of().to_vec()),
            state_of: Arc::new(layer.state_of().to_vec()),
            colours: layer.colours().collect(),
            demilitarized,
        }
    }

    pub fn regions_and_zone(&self, map: &content_reader_map::map::MapData) -> (Regions, Zone) {
        let field = |values: &Arc<Vec<u32>>| province_field::ProvinceField(Arc::new(province_field::Storage::Dense(Arc::clone(values))));
        let regions = Regions {
            country_of: field(&self.country_of),
            owner_of: field(&self.owner_of),
            state_of: field(&self.state_of),
            country_colors: ValueIndex::try_from_iter(self.colours.iter().copied()).unwrap_or_default(),
            warnings: Vec::new(),
            strategic_attributes: Default::default(),
            mode: None,
        };
        let zone = Zone::new(map, &regions, &self.demilitarized);
        (regions, zone)
    }
}
