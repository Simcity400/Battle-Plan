//! Drawn land counter observer inputs.
use crate::map::gui_resources::contains;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub next: Option<i64>,
    pub kind: Option<u32>,
}

/// The counter child observed by its tooltip handler.
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
    /// The active-ability item's own handler, keyed by its bound record.
    Ability {
        country: u32,
        ordinal: u32,
    },
}

/// A drawn land counter's native tooltip inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct CounterHover {
    pub control: CounterControl,
    /// The actually drawn GUI object's fallback, evaluated only if its handler declines.
    pub authored: Option<super::gui_authored::Authored>,
    /// The counter's rectangle, in the interface pixels the hit tests use.
    pub rect: [f32; 4],
    /// Estimated fixed5: the representative's whole route for ours, next province otherwise.
    pub remaining: Option<i64>,
    /// Whether the viewing country owns the stack, which picks the wording.
    pub ours: bool,
    pub coarse: bool,
    /// Whether the counter is hidden by intel; foreign hidden counters skip arrival text.
    pub hidden: bool,
    /// Railway-gun counter branch.
    pub railway_gun: bool,
    /// Every admitted member, including members below full intel, in counter order.
    pub members: Arc<[u64]>,
    /// Per-member timing in the same order as `members`.
    pub timing: Arc<[Timing]>,
}

impl CounterHover {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        contains(self.rect, point)
    }
}
