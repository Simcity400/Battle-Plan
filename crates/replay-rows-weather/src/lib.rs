use engine_allocation as allocation;
use replay_rows::codec::{Decoder, invalid_static as invalid, put_varint};
use replay_rows::rows::*;
use replay_rows::value::Value;
use replay_rows::{typed_codec, value_struct};
use replay_rows_world::{RegionCombatWeather, WeatherRegionAttackInputs};
use std::io;

mod naval_weather;
pub use naval_weather::*;
pub mod temperature;
