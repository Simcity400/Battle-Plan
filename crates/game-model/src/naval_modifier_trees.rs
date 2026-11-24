#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Country = 0,
    Pride = 1,
    Experience = 2,
    Medals = 3,
    Doctrine = 4,
    Terrain = 5,
    Weather = 6,
    Night = 7,
    TaskForce = 8,
    Commander = 9,
    CommanderClass = 10,
    Critical = 11,
    Support0 = 12,
    Support1 = 13,
    Support2 = 14,
    Support3 = 15,
    SideTerrain = 16,
    SideWeather = 17,
}

impl Kind {
    pub const ALL: [Self; 18] = [
        Self::Country,
        Self::Pride,
        Self::Experience,
        Self::Medals,
        Self::Doctrine,
        Self::Terrain,
        Self::Weather,
        Self::Night,
        Self::TaskForce,
        Self::Commander,
        Self::CommanderClass,
        Self::Critical,
        Self::Support0,
        Self::Support1,
        Self::Support2,
        Self::Support3,
        Self::SideTerrain,
        Self::SideWeather,
    ];

    pub const fn from_u8(value: u8) -> Option<Self> {
        if (value as usize) < Self::ALL.len() {
            Some(Self::ALL[value as usize])
        } else {
            None
        }
    }

    pub const fn carried(self) -> bool {
        matches!(self, Self::Country | Self::Commander)
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SideSource {
    Country = 0,
    Weather = 1,
    Terrain = 2,
    Commander = 3,
    OpposingCommander = 4,
}

impl SideSource {
    pub const ALL: [Self; 5] = [
        Self::Country,
        Self::Weather,
        Self::Terrain,
        Self::Commander,
        Self::OpposingCommander,
    ];

    pub const fn from_ordinal(value: u16) -> Option<Self> {
        if (value as usize) < Self::ALL.len() {
            Some(Self::ALL[value as usize])
        } else {
            None
        }
    }
}

pub type Owner = (Kind, u64, u32);

pub fn ship_owner(
    scope: u8,
    index: u32,
    selector: u32,
    reference: u64,
    ship: u64,
    task_force: u64,
    country: u32,
    commander: u64,
    ship_class: u32,
    subunit: u32,
) -> Option<Owner> {
    Some(match scope {
        0 => (Kind::Country, u64::from(country), 0),
        1 => (Kind::Pride, 0, 0),
        2 => (Kind::Experience, ship, 0),
        3 => (Kind::Medals, ship, 0),
        4 => (Kind::Doctrine, task_force, selector),
        5 => (Kind::Terrain, task_force, ship_class),
        6 => (Kind::Weather, task_force, 0),
        7 => (Kind::Night, task_force, 0),
        8 => (Kind::TaskForce, task_force, 0),
        9 => (Kind::Commander, commander, 0),
        10 => (Kind::CommanderClass, commander, subunit),
        11 => (Kind::Critical, ship, index),
        12 => (Kind::Support0, reference, 0),
        13 => (Kind::Support1, reference, 0),
        14 => (Kind::Support2, reference, 0),
        15 => (Kind::Support3, reference, 0),
        _ => return None,
    })
}

pub fn side_owner(
    source: SideSource,
    combat: u64,
    country: Option<u32>,
    commander: u64,
    opposing_commander: u64,
) -> Option<Owner> {
    let country = country?;
    Some(match source {
        SideSource::Country => (Kind::Country, u64::from(country), 0),
        SideSource::Weather => (Kind::SideWeather, combat, 0),
        SideSource::Terrain => (Kind::SideTerrain, combat, 0),
        SideSource::Commander if commander != 0 => (Kind::Commander, commander, 0),
        SideSource::OpposingCommander if opposing_commander != 0 => {
            (Kind::Commander, opposing_commander, 0)
        }
        SideSource::Commander | SideSource::OpposingCommander => return None,
    })
}
