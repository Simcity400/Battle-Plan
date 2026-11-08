//! Actual drawn selection hits shared by grid and hover consumers.
use crate::map::gui_resources::contains;
use crate::unit_handle::Unit;
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

/// A selectable plate actually drawn this frame, in interface pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub ours: bool,
    /// Port plates are composed with base GUI windows, before raid icons.
    pub port: bool,
    pub rect: [f32; 4],
    pub kind: Kind,
    pub members: Arc<[u64]>,
    pub layer: u64,
    /// The drawn `count_bg` the pointer's button state applies to; `None` on a part that only
    /// forwards its counter's selection.
    pub plate: Option<[f32; 4]>,
}

impl Hit {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        contains(self.rect, point)
    }

    /// Whether `point` is on the counter's `count_bg` itself.
    pub fn on_plate(&self, point: [f32; 2]) -> bool {
        self.plate.is_some_and(|plate| contains(plate, point))
    }

    pub fn units(&self) -> impl Iterator<Item = Unit> + '_ {
        self.members.iter().map(|&handle| self.kind.unit(handle))
    }
}

/// A drawn base GUI object which intercepts clicks without selecting units.
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
