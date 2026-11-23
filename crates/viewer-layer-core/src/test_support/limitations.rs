use super::recorded::record;
use crate::playhead_host::units::Units;
use replay_columnar::manifest::Status;
use replay_data::rows::TableId;

pub const REASON: &str = "the collector could not read it";

pub fn unavailable(name: &str) -> Units {
    record(name, 12, |at, store| {
        let reason = store.intern(REASON.as_bytes()).unwrap();
        let status = if (4..8).contains(&at) { Status::Unavailable { reason } } else { Status::Complete };
        store.status(TableId::LandCombats.index(), status, true, Some(at as i32), true).unwrap();
        store.status(TableId::Countries.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    })
}
