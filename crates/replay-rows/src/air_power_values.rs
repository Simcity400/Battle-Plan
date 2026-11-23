use crate::value::value_struct;

pub use game_model::air_power::{Contribution as AirPowerContribution, Menu as AirPowerMenu};
value_struct!(AirPowerContribution {
    country: u32,
    amount: i64
});
value_struct!(AirPowerMenu { coverage: i64, reductions: [i64; 4], contributions: [Vec<AirPowerContribution>; 2] });
opaque_value!(Option<std::sync::Arc<AirPowerMenu>>);
