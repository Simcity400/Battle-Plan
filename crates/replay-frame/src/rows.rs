pub use crate::fuel_awards::with_award_recipients;
pub use crate::fuel_demand::with_demand_parts;
pub use replay_data::rows::*;

#[cfg(test)]
use crate::{
    codec::{Decoder, put_bytes},
    format::FormatVersion,
    value::Value,
};

#[cfg(test)]
#[path = "frame/row_contract_tests/event_feed.rs"]
mod event_feed_frame_tests;
#[cfg(test)]
#[path = "frame/row_contract_tests/format98.rs"]
pub(crate) mod format98_tests;
#[cfg(test)]
#[path = "frame/row_contract_tests/domains.rs"]
mod frame_domain_tests;
#[cfg(test)]
#[path = "frame/row_contract_tests/packet_hour.rs"]
mod packet_hour_tests;
#[cfg(test)]
#[path = "frame/row_contract_tests/politics.rs"]
mod politics_frame_tests;
#[cfg(test)]
#[path = "frame/row_contract_tests/slice_update.rs"]
mod slice_update_frame_tests;
#[cfg(test)]
#[path = "frame/row_contract_tests/template_display_name.rs"]
mod template_display_name_tests;
