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

const LAND: &[&str] = &[
    "armor",
    "motorized",
    "mechanized",
    "infantry",
    "train",
    "land_cruiser",
];
const AIR: &[&str] = &[
    "fighter",
    "interceptor",
    "tactical_bomber",
    "strategic_bomber",
    "cas",
    "naval_bomber",
    "missile",
    "suicide",
    "scout_plane",
    "air_transport",
    "maritime_patrol_plane",
    "heavy_fighter",
    "emplacement_gun_ammo",
    "ballistic_missile",
    "nuclear_missile",
    "sam_missile",
];
const NAVAL: &[&str] = &[
    "convoy",
    "capital_ship",
    "submarine",
    "screen_ship",
    "floating_harbor",
    "carrier",
    "support_ship",
];
const SHIP_DESIGNER: &[&str] = &[
    "capital_ship",
    "submarine",
    "screen_ship",
    "carrier",
    "support_ship",
];
const TANK_DESIGNER: &[&str] = &["armor"];
const PLANE_DESIGNER: &[&str] = &[
    "fighter",
    "interceptor",
    "tactical_bomber",
    "strategic_bomber",
    "cas",
    "naval_bomber",
    "suicide",
    "scout_plane",
    "maritime_patrol_plane",
    "heavy_fighter",
    "ballistic_missile",
    "nuclear_missile",
    "sam_missile",
];

static FAMILIES: RuntimeTable<CategoryFamilies> = RuntimeTable::new(resolve);

fn resolve() -> Verified<CategoryFamilies> {
    let names = auxiliary_part(|tables| &tables.category_names)
        .map_err(|withheld| withheld.requires("equipment category families"))?;
    let bits = |members: &[&str]| {
        names
            .iter()
            .filter(|(name, _)| members.contains(name))
            .fold(0, |bits, (_, bit)| bits | bit)
    };
    Ok(CategoryFamilies {
        land: bits(LAND),
        air: bits(AIR),
        naval: bits(NAVAL),
        ship_designer: bits(SHIP_DESIGNER),
        tank_designer: bits(TANK_DESIGNER),
        plane_designer: bits(PLANE_DESIGNER),
    })
}

pub fn category_families() -> Result<&'static CategoryFamilies, String> {
    FAMILIES.get().map_err(ToString::to_string)
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
