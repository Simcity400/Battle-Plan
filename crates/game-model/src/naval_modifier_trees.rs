/// The kind of a source's owner identity (`replay_data::naval_trees::Owner`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A raw country reference: the side's first primary country or a ship's
    /// task-force roster owner (scope 0).
    Country = 0,
    /// The static pride_of_the_fleet modifier (scope 1); owner zero.
    Pride = 1,
    /// A ship handle: its experience modifier (scope 2).
    Experience = 2,
    /// A ship handle: its medal store (scope 3).
    Medals = 3,
    /// A task-force handle; sub is the doctrine map key (scope 4).
    Doctrine = 4,
    /// A task-force handle; sub is the ship class (scope 5).
    Terrain = 5,
    /// A task-force handle: its location's region weather (scope 6).
    Weather = 6,
    /// A task-force handle: its location's night modifier (scope 7).
    Night = 7,
    /// A task-force handle (scope 8).
    TaskForce = 8,
    /// A leader handle (scope 9 and the side commanders).
    Commander = 9,
    /// A leader handle; sub is the ship's subunit (scope 10).
    CommanderClass = 10,
    /// A ship handle; sub is the critical's index (scope 11).
    Critical = 11,
    /// An HQ influence's character reference, one per support lane
    /// (scopes 12-15).
    Support0 = 12,
    Support1 = 13,
    Support2 = 14,
    Support3 = 15,
    /// A combat handle: the combat province's terrain (side source).
    SideTerrain = 16,
    /// A combat handle: the combat region's weather (side source).
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

    /// The kinds whose every writer is hooked (`naval_tree_mutations`), so a
    /// held tree is carried while its receiver's certificate holds.
    pub const fn carried(self) -> bool {
        matches!(self, Self::Country | Self::Commander)
    }
}

/// The side hover sources, in its visiting order, then the opposing commander
/// that 0x5A reads in place of the own one.
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

/// A source's identity: kind, owner and sub-key.
pub type Owner = (Kind, u64, u32);

/// The recorded identity of one ship modifier source of a ship, from what
/// NavalCombatShips records of it and of the ship; `None` for a scope that
/// carries no tree. The accumulators behind each scope are those
/// [`naval_ship_modifier_inputs::visit_sources`] reads: the task force's
/// country, the pride static, the ship's experience and medals, the task
/// force's doctrine entry, location terrain (for the ship's class), region
/// weather and province night, the task force, the commander and its class
/// selection, the ship's critical and an HQ influence's support definitions.
#[allow(
    clippy::too_many_arguments,
    reason = "every recorded identity the join reads is a separate field"
)]
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

/// The recorded identity of one side source: `None` without a primary
/// country (the side reaches nothing) or without the commander it reads.
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
