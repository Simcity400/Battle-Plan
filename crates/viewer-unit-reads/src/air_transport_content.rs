use content_reader_map::province_geometry::ProvinceGeometry;
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub struct AirTransportContent<'a> {
    pub geometry: &'a ProvinceGeometry,
    pub state_provinces: &'a BTreeMap<u32, Vec<u32>>,
    pub region_by_province: &'a BTreeMap<u32, u32>,
    pub region_provinces: &'a BTreeMap<u32, Vec<u32>>,
    pub sub_units: Result<&'a content_reader_script::sub_units::SubUnits, &'a str>,
    pub map_scale: i64,
    pub ace_wing_size: i64,
    pub ace_wing_size_max_bonus: i64,
}

impl AirTransportContent<'_> {
    pub fn target_region(&self, province: u32) -> Option<&[u32]> {
        self.region_by_province
            .get(&province)
            .and_then(|region| self.region_provinces.get(region))
            .map(Vec::as_slice)
    }

    pub fn transport_pool(&self, index: u32) -> Result<bool, String> {
        if index == 0 {
            return Ok(false);
        }
        let units = self.sub_units.map_err(str::to_owned)?;
        let name = units.name(index).ok_or_else(|| {
            format!("pool sub-unit index {index} is not in the recorded content")
        })??;
        let bit = content_reader_script::equipment_stats::category_mask("air_transport")
            .expect("air_transport category is defined");
        Ok(units.category(name.as_bytes())? & bit != 0)
    }
}
