//! Native unit classes shared by settled movement and world entity history.

/// Unit class governing stack kinds, anchor slots and snapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CounterClass {
    #[default]
    Division,
    /// Class 13.
    RailwayGun,
    /// Class 1, a navy's task force.
    TaskForce,
}

impl CounterClass {
    /// The slot for the class in a province: `Standstill` (0) for a division and
    /// `Standstill RG` (21) for a railway gun whatever the province, and for a task force
    /// `Ship in port` (19) in a land province (province-kind bit 0), else 0. The standing
    /// counter and every province of the animator's route take it. The slot uses the
    /// unit's own class, so a transported land unit keeps its slot.
    pub const fn standstill(self, land: bool) -> u32 {
        match self {
            Self::Division => 0,
            Self::RailwayGun => 21,
            Self::TaskForce if land => 19,
            Self::TaskForce => 0,
        }
    }

    /// The first of the eight directional kinds, from land or from sea. A task force takes the
    /// division's land kinds from either.
    pub const fn directional(self, from_land: bool) -> u32 {
        match (self, from_land) {
            (Self::Division, true) | (Self::TaskForce, _) => 1,
            (Self::Division, false) => 11,
            (Self::RailwayGun, true) => 22,
            (Self::RailwayGun, false) => 30,
        }
    }
}
