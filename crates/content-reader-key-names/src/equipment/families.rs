use engine_executable::{
    availability::Verified,
    executable_tables::{RuntimeTable, auxiliary_part},
    native_values::dlc_feature::{BY_BLOOD_ALONE, MAN_THE_GUNS, NO_STEP_BACK},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModularDesigner {
    Ship,
    Tank,
    Plane,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CategoryFamilies {
    pub land: u64,
    pub air: u64,
    pub naval: u64,
    ship_designer: u64,
    tank_designer: u64,
    plane_designer: u64,
}

static FAMILIES: RuntimeTable<CategoryFamilies> = RuntimeTable::new(resolve);

fn resolve() -> Verified<CategoryFamilies> {
    let recorded = auxiliary_part(|tables| &tables.category_families)
        .map_err(|withheld| withheld.requires("equipment category families"))?;
    Ok(CategoryFamilies {
        land: recorded.land,
        air: recorded.air,
        naval: recorded.naval,
        ship_designer: recorded.ship_designer,
        tank_designer: recorded.tank_designer,
        plane_designer: recorded.plane_designer,
    })
}

pub fn category_families() -> Result<&'static CategoryFamilies, String> {
    FAMILIES.get().map_err(ToString::to_string)
}

pub fn modular_categories(dlc_mask: u32) -> Result<u64, String> {
    if dlc_mask & (MAN_THE_GUNS | NO_STEP_BACK | BY_BLOOD_ALONE) == 0 {
        return Ok(0);
    }
    Ok(category_families()?.modular_categories(dlc_mask))
}

impl CategoryFamilies {
    pub fn modular_categories(&self, dlc_mask: u32) -> u64 {
        self.designers(dlc_mask)
            .into_iter()
            .fold(0, |bits, (_, designer)| bits | designer)
    }

    pub fn modular_designer(&self, category: u64, dlc_mask: u32) -> Option<ModularDesigner> {
        self.designers(dlc_mask)
            .into_iter()
            .find(|&(_, designer)| category & designer != 0)
            .map(|(kind, _)| kind)
    }

    pub fn has_modular_designer(&self, category: u64) -> bool {
        category & (self.ship_designer | self.tank_designer | self.plane_designer) != 0
    }

    fn designers(&self, dlc_mask: u32) -> [(ModularDesigner, u64); 3] {
        let enabled = |dlc: u32, bits: u64| if dlc_mask & dlc != 0 { bits } else { 0 };
        [
            (ModularDesigner::Ship, enabled(MAN_THE_GUNS, self.ship_designer)),
            (ModularDesigner::Tank, enabled(NO_STEP_BACK, self.tank_designer)),
            (ModularDesigner::Plane, enabled(BY_BLOOD_ALONE, self.plane_designer)),
        ]
    }
}
