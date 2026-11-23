use super::gui_authored::Authored;
use super::unit_hits as unit_selection;
use viewer_runtime::geometry::contains;
use crate::native_gui::HitGeometry;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Control {
    #[default]
    Normal,
    InvalidOrders,
    NoOrders,
    Supply,
    Carrier,
    Pride,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hover {
    pub control: Control,
    pub unit_hit: usize,
    pub rect: [f32; 4],
    pub hit: Option<HitGeometry>,
    pub authored: Option<Authored>,
    pub supported: bool,
    pub ours: bool,
    pub coarse: bool,
    pub hidden: bool,
    pub members: Arc<[u64]>,
}

impl Hover {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        self.hit
            .as_ref()
            .map_or_else(|| contains(self.rect, point), |hit| hit.contains(point))
    }

    pub fn matches(&self, unit: &unit_selection::Hit) -> bool {
        !unit.port && unit.kind == unit_selection::Kind::TaskForce && unit.members == self.members
    }
}
