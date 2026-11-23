use engine_allocation as allocation;
use replay_rows::codec::{Decoder, invalid_static as invalid, put_varint};
use replay_rows::rows::*;
use replay_rows::typed_codec;
use replay_rows::value::Value;
use replay_rows::{opaque_value, value_struct};
use std::io;

mod combat_log;
pub use combat_log::*;
mod economy_construction;
pub use economy_construction::*;
mod equipment_destruction;
pub use equipment_destruction::*;
mod fuel_demand;
pub use fuel_demand::*;
mod industrial_organisations;
pub use industrial_organisations::*;
mod market;
pub use market::*;
mod production_modifier_rows;
pub use production_modifier_rows::*;
#[cfg(test)]
mod ranges_tests;

pub use schema::*;
mod schema;
