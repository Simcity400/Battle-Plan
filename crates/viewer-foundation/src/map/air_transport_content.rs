//! Borrowed static inputs for native air transport admission.
use installed_content_core::province_geometry::ProvinceGeometry;
use std::collections::BTreeMap;

/// The static content reached by long paradrop transport labels. Borrowed
/// from the ordinary assets; no second content parse or per-order copy.
#[derive(Clone, Copy)]
pub struct AirTransportContent<'a> {
    pub geometry: &'a ProvinceGeometry,
    pub state_provinces: &'a BTreeMap<u32, Vec<u32>>,
    pub region_by_province: &'a BTreeMap<u32, u32>,
    pub region_provinces: &'a BTreeMap<u32, Vec<u32>>,
    pub sub_units: Result<&'a installed_content_core::unit_leader::SubUnits, &'a str>,
    pub map_scale: i64,
    pub ace_wing_size: i64,
    pub ace_wing_size_max_bonus: i64,
}

impl AirTransportContent<'_> {
    /// The destination's nullable strategic region.
    pub fn target_region(&self, province: u32) -> Option<&[u32]> {
        self.region_by_province
            .get(&province)
            .and_then(|region| self.region_provinces.get(region))
            .map(Vec::as_slice)
    }

    /// Only the authored `air_transport` bit is needed. Null subtype index zero
    /// has no transport bit; uncertain content remains a refusal.
    pub fn transport_pool(&self, index: u32) -> Result<bool, String> {
        if index == 0 {
            return Ok(false);
        }
        let units = self.sub_units.map_err(str::to_owned)?;
        let name = units.name(index).ok_or_else(|| {
            format!("pool sub-unit index {index} is not in the recorded content")
        })??;
        let bit = installed_content_core::equipment::category_mask("air_transport")
            .expect("air_transport category is defined");
        Ok(units.category(name.as_bytes())? & bit != 0)
    }
}
