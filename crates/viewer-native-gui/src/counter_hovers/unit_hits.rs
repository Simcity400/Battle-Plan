use viewer_runtime::geometry::contains;
use viewer_runtime::unit_handle::Unit;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Division,
    RailwayGun,
    TaskForce,
}

impl Kind {
    pub const fn unit(self, handle: u64) -> Unit {
        match self {
            Self::Division => Unit::Division(handle),
            Self::RailwayGun => Unit::RailwayGun(handle),
            Self::TaskForce => Unit::TaskForce(handle),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub ours: bool,
    pub port: bool,
    pub rect: [f32; 4],
    pub kind: Kind,
    pub members: Arc<[u64]>,
    pub layer: u64,
    pub plate: Option<[f32; 4]>,
}

impl Hit {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        contains(self.rect, point)
    }

    pub fn on_plate(&self, point: [f32; 2]) -> bool {
        self.plate.is_some_and(|plate| contains(plate, point))
    }

    pub fn units(&self) -> impl Iterator<Item = Unit> + '_ {
        self.members.iter().map(|&handle| self.kind.unit(handle))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blocker {
    pub rect: [f32; 4],
    pub layer: u64,
}

impl Blocker {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        contains(self.rect, point)
    }
}
