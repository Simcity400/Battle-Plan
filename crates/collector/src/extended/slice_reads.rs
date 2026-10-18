//! SliceReads: the hour each owner's rows of a sliced table were read at.
//!
//! Nothing is read from the game for it. The spool assembles the table from
//! each hour's sliced reads while it merges them (`Campaign::merge_sliced_inputs`),
//! so this pack only reserves the table's place in the frame.
use engine_allocation::string;
use engine_native::walk::Outcome;
use replay_data::rows::TableId;
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::SliceReads];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// The spool replaces this placeholder with the assembled table.
pub(crate) const ASSEMBLED_BY_SPOOL: &str = "assembled by the spool from the hour's sliced reads";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities;

pub struct Captured;
impl Captured {
    pub fn with_capacities(_capacity: &Capacities) -> Self {
        Self
    }
    pub fn reset(&mut self) {}
    pub fn allocated_bytes(&self) -> usize {
        0
    }
    pub fn grow(&mut self, _capacity: &mut Capacities) {}
    pub fn outcomes(&self) -> [Outcome; N] {
        [Outcome::Unavailable {
            reason: ASSEMBLED_BY_SPOOL,
        }]
    }
    pub fn tables(&self) -> [TableView; N] {
        [TableView::Unavailable {
            reason: string(ASSEMBLED_BY_SPOOL).unwrap_or_default(),
        }]
    }
}
