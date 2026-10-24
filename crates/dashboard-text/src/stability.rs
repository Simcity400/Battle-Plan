//! Displayed country stability, war support and command power from recorded inputs.

use replay_data::rows::CountryCommandPowerRow;

pub use replay_data::country_stability::{
    Terms, fixed5_to_q15, interception_term, max_command_power, popularity_term, q15_to_fixed5,
    stability, war_support,
};

/// The CountryCommandPower row of the country at canonical `slot` in the
/// same frame (the stability row's native country ID mapped through
/// CountryLinks); `None` when the table is not complete or has no row.
pub fn command_power(
    decoded: &dyn replay_frame::frame::DecodedSource,
    slot: u32,
) -> Option<&CountryCommandPowerRow> {
    let rows = crate::triggers::refusal::FrameInputs::trigger_table::<
        replay_data::rows::tables::CountryCommandPower,
    >(decoded)
    .ok()?;
    crate::triggers::refusal::reference(
        rows.binary_search_by_key(&slot, |row| row.canonical_slot)
            .ok()
            .map(|at| &rows[at]),
        slot != 0,
        || {
            crate::triggers::refusal::missing_reference(
                replay_data::rows::TableId::CountryCommandPower,
                Some(crate::triggers::Store::Country(slot)),
            )
        },
    )
}
