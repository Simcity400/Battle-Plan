use crate::equipment;
pub use content_reader_script::sub_units::{SubUnit, SubUnits, sub_unit_files, sub_units};

pub trait AirWings {
    fn air_carrier_capable(
        &self,
        name: &[u8],
        equipment: &equipment::Database,
    ) -> Result<bool, String>;
    fn air_wing_sizes(
        &self,
        name: &[u8],
        equipment: &equipment::Database,
    ) -> Result<[i32; 4], String>;
}

impl AirWings for SubUnits {
    fn air_carrier_capable(
        &self,
        name: &[u8],
        equipment: &equipment::Database,
    ) -> Result<bool, String> {
        let wing = self.air_wing(name)?;
        match &wing.first_requirement {
            None => Ok(false),
            Some(required) => equipment
                .named(required)
                .map(|item| item.carrier_capable)
                .ok_or_else(|| {
                    format!(
                        "sub-unit {} requires unknown equipment {}",
                        String::from_utf8_lossy(name),
                        String::from_utf8_lossy(required)
                    )
                }),
        }
    }
    fn air_wing_sizes(
        &self,
        name: &[u8],
        equipment: &equipment::Database,
    ) -> Result<[i32; 4], String> {
        let wing = self.air_wing(name)?;
        let categories = self.category(name)?;
        if categories & 0x1f_0037_fc00 == 0 {
            return Ok([0; 4]);
        }
        let mut sizes = wing.sizes;
        if let Some(required) = &wing.first_requirement {
            let required = equipment.named(required).ok_or_else(|| {
                format!(
                    "sub-unit {} requires unknown equipment {}",
                    String::from_utf8_lossy(name),
                    String::from_utf8_lossy(required)
                )
            })?;
            if sizes[0] == 0 {
                sizes[0] = 100;
            }
            if required.carrier_capable {
                if sizes[1] == 0 {
                    sizes[1] = 10;
                }
            } else if categories & 0x1c_0001_0000 == 0 {
                sizes[1] = 0;
            }
        }
        Ok(sizes)
    }
}
