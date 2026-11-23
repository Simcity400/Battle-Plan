use viewer_runtime::geometry::contains;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub next: Option<i64>,
    pub kind: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CounterControl {
    #[default]
    Normal,
    InvalidOrders,
    Attrition,
    Supply,
    DefendingArea,
    DigIn,
    Action,
    NoOrders,
    LeaderEnroute,
    Ability {
        country: u32,
        ordinal: u32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CounterHover {
    pub control: CounterControl,
    pub authored: Option<super::gui_authored::Authored>,
    pub rect: [f32; 4],
    pub remaining: Option<i64>,
    pub ours: bool,
    pub coarse: bool,
    pub hidden: bool,
    pub railway_gun: bool,
    pub members: Arc<[u64]>,
    pub timing: Arc<[Timing]>,
}

impl CounterHover {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        contains(self.rect, point)
    }
}
