
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CounterClass {
    #[default]
    Division,
    RailwayGun,
    TaskForce,
}

impl CounterClass {
    pub const fn standstill(self, land: bool) -> u32 {
        match self {
            Self::Division => 0,
            Self::RailwayGun => 21,
            Self::TaskForce if land => 19,
            Self::TaskForce => 0,
        }
    }

    pub const fn directional(self, from_land: bool) -> u32 {
        match (self, from_land) {
            (Self::Division, true) | (Self::TaskForce, _) => 1,
            (Self::Division, false) => 11,
            (Self::RailwayGun, true) => 22,
            (Self::RailwayGun, false) => 30,
        }
    }
}
