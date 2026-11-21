
use replay_data::rows::CountryCommandPowerRow;

pub use replay_data::country_stability::{
    Terms, fixed5_to_q15, interception_term, max_command_power, popularity_term, q15_to_fixed5,
    stability, war_support,
};

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

pub fn command_power_in(
    frame: replay_frame::frame::HourFrame<'_>,
    slot: u32,
    mask: u64,
) -> Option<std::borrow::Cow<'_, CountryCommandPowerRow>> {
    use replay_data::rows::{TableSchema, tables::CountryCommandPower};
    let rows = frame
        .complete::<CountryCommandPower>(mask)
        .map_err(|why| {
            crate::triggers::Failure::input(CountryCommandPower::ID, "", None, why.to_string())
                .resolution_failed();
        })
        .ok()?;
    crate::triggers::refusal::reference(
        rows.get(u128::from(slot)).ok().flatten(),
        slot != 0,
        || {
            crate::triggers::refusal::missing_reference(
                replay_data::rows::TableId::CountryCommandPower,
                Some(crate::triggers::Store::Country(slot)),
            )
        },
    )
}
