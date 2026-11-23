
use super::{
    Capacities, Memory, ccountry, cdecision_status, cdynamic_modifier_container, cpolitical_party,
};
use crate::country_scale::{raise, rows};

pub(super) fn presize(
    capacity: &mut Capacities,
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<(), &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let mut parties = cpolitical_party::Demand::default();
    let mut modifiers = cdynamic_modifier_container::Demand::default();
    let mut refusal = None;
    capacity.raise_for_countries(countries.count() as usize);
    let mut focus_rows = [0_usize; 2];
    let mut focus_bytes = 0_usize;
    let mut slots = 0_usize;
    let mut leaders = 0_usize;
    let mut ideas = engine_native_politics::cpolitical_status::IdeaDemand::default();
    let mut decisions = [0_usize; 2];
    let mut tactic_bytes = 0_usize;
    let mut research_rows = [0_usize; 3];
    let research_context = engine_research::research_inputs::Context::read(memory, base);
    let modifier_context = engine_research::research_modifiers::Context::read(memory, base);
    let mut research_inputs = Vec::new();
    for slot in 1..countries.count() {
        let country = match countries.at(slot) {
            Ok(Some(country)) => country,
            Ok(None) => continue,
            Err(reason) => {
                refusal.get_or_insert(reason);
                continue;
            }
        };
        let focus = engine_focus::cnational_focus_progress::count(memory, base, country.pointer, country.id);
        for (index, outcome) in [focus.outcome, focus.completed_outcome]
            .into_iter()
            .enumerate()
        {
            if let Err(reason) = add_rows(&mut focus_rows[index], outcome) {
                refusal.get_or_insert(reason);
            }
        }
        focus_bytes = focus_bytes
            .checked_add(focus.required_bytes)
            .ok_or("focus sizing byte count overflow")?;
        let slot_demand = engine_research::ctechnology_status::count_slots(memory, base, country.pointer, country.id);
        if let Err(reason) = add_rows(&mut slots, slot_demand) {
            refusal.get_or_insert(reason);
        }
        match engine_native_politics::cpolitical_status::Reader::new(memory, base, country.pointer) {
            Ok(reader) => {
                if let Err(reason) = add_rows(
                    &mut leaders,
                    reader.party_leaders_buffer(country.id, engine_native::walk::Buffer::Count),
                ) {
                    refusal.get_or_insert(reason);
                }
                let active = reader.ideas_buffer(engine_native::walk::Buffer::Count, engine_native::walk::Buffer::Count);
                if let Err(reason) = add_rows(&mut ideas.rows, active.outcome)
                {
                    refusal.get_or_insert(reason);
                }
                ideas.bytes = ideas
                    .bytes
                    .checked_add(active.required_bytes)
                    .ok_or("active idea sizing byte count overflow")?;
            }
            Err(reason) => {
                refusal.get_or_insert(reason);
            }
        }
        let decision = cdecision_status::count(memory, base, country.pointer);
        if let Err(reason) = add_rows(&mut decisions[0], decision.outcome) {
            refusal.get_or_insert(reason);
        }
        decisions[1] = decisions[1]
            .checked_add(decision.required_bytes)
            .ok_or("decision sizing byte count overflow")?;
        let tactic = engine_native::ccombat_tactic::count_preferred(memory, base, country.pointer, country.id);
        if let Err(reason) = match tactic.outcome {
            engine_native::walk::Outcome::Unavailable { reason } => Err(reason),
            _ => Ok(()),
        } {
            refusal.get_or_insert(reason);
        }
        tactic_bytes = tactic_bytes
            .checked_add(tactic.required_token_bytes)
            .ok_or("preferred tactic sizing byte count overflow")?;
        let mut local_slots = 0;
        let prepared = add_rows(&mut local_slots, slot_demand).and_then(|()| {
            research_inputs
                .try_reserve(local_slots.saturating_sub(research_inputs.len()))
                .map_err(|_| "research sizing scratch allocation failed")?;
            research_inputs
                .resize_with(local_slots, engine_research::research_inputs::Row::default);
            Ok(())
        });
        let (input, bonuses) = match prepared {
            Ok(()) => engine_research::research_inputs::walk_country_buffer(
                memory,
                base,
                country.pointer,
                country.id,
                research_context,
                engine_native::walk::Buffer::Growing(&mut research_inputs),
                engine_native::walk::Buffer::Count,
            ),
            Err(reason) => (
                engine_native::walk::Outcome::Unavailable { reason },
                engine_native::walk::Outcome::Unavailable { reason },
            ),
        };
        for (index, outcome) in [input, bonuses].into_iter().enumerate() {
            if let Err(reason) = add_rows(&mut research_rows[index], outcome)
            {
                refusal.get_or_insert(reason);
            }
        }
        if let engine_native::walk::Outcome::Rows { len } = input {
            let modifiers = engine_research::research_modifiers::walk_country_buffer(
                memory,
                base,
                &countries,
                country,
                modifier_context,
                &research_inputs[..len],
                engine_native::walk::Buffer::Count,
            );
            if let Err(reason) = add_rows(&mut research_rows[2], modifiers) {
                refusal.get_or_insert(reason);
            }
        }
        let party = cpolitical_party::demand(memory, base, country.pointer).and_then(|next| {
            let next = cpolitical_party::Demand {
                rows: parties
                    .rows
                    .checked_add(next.rows)
                    .ok_or("political party sizing row count overflow")?,
                bytes: parties
                    .bytes
                    .checked_add(next.bytes)
                    .ok_or("political party sizing byte count overflow")?,
            };
            parties = next;
            Ok(())
        });
        if let Err(reason) = party {
            refusal.get_or_insert(reason);
        }
        let modifier =
            cdynamic_modifier_container::demand(memory, base, country.pointer, country.id)
                .and_then(|next| {
                    let next = cdynamic_modifier_container::Demand {
                        rows: modifiers
                            .rows
                            .checked_add(next.rows)
                            .ok_or("dynamic modifier sizing row count overflow")?,
                        bytes: modifiers
                            .bytes
                            .checked_add(next.bytes)
                            .ok_or("dynamic modifier sizing byte count overflow")?,
                        values: modifiers
                            .values
                            .checked_add(next.values)
                            .ok_or("dynamic modifier sizing value count overflow")?,
                    };
                    modifiers = next;
                    Ok(())
                });
        if let Err(reason) = modifier {
            refusal.get_or_insert(reason);
        }
    }
    for (index, count) in [
        (0, focus_rows[0]),
        (1, focus_rows[1]),
        (3, slots),
        (7, ideas.rows),
        (9, decisions[0]),
        (18, research_rows[0]),
        (19, research_rows[1]),
        (20, research_rows[2]),
        (21, leaders),
    ] {
        raise(&mut capacity.rows[index], rows(count, 1));
    }
    for (target, count) in [
        (&mut capacity.focus_text, focus_bytes),
        (&mut capacity.idea_text, ideas.bytes),
        (&mut capacity.idea_countries, ideas.rows),
        (&mut capacity.decision_text, decisions[1]),
        (&mut capacity.decision_countries, decisions[0]),
        (&mut capacity.tactic_text, tactic_bytes),
    ] {
        raise(target, rows(count, 1));
    }
    match engine_native_politics::cpolitical_status::timed_idea_demand(
        memory, base, state,
    ) {
        Ok(timed) => {
            raise(&mut capacity.rows[23], rows(timed.rows, 1));
            raise(&mut capacity.timed_idea_text, rows(timed.bytes, 1));
        }
        Err(reason) => {
            refusal.get_or_insert(reason);
        }
    }
    let sharing =
        engine_research::ctechnology_sharing_group::count(memory, base, state);
    for (index, outcome) in [(12, sharing.groups), (13, sharing.members)] {
        let mut count = 0;
        if let Err(reason) = add_rows(&mut count, outcome) {
            refusal.get_or_insert(reason);
        }
        raise(&mut capacity.rows[index], rows(count, 1));
    }
    let balances = engine_power_balance::power_balance_manager::walk(
        memory,
        base,
        state,
        engine_power_balance::power_balance_manager::Buffers {
            records: engine_native::walk::Buffer::Count,
            members: engine_native::walk::Buffer::Count,
            ranges: engine_native::walk::Buffer::Count,
            overrides: engine_native::walk::Buffer::Count,
            text: engine_native::walk::Buffer::Count,
        },
    );
    for (index, outcome) in [
        (14, balances.records),
        (15, balances.members),
        (16, balances.ranges),
        (17, balances.overrides),
    ] {
        let mut count = 0;
        if let Err(reason) = add_rows(&mut count, outcome) {
            refusal.get_or_insert(reason);
        }
        raise(&mut capacity.rows[index], rows(count, 1));
    }
    raise(&mut capacity.balance_text, rows(balances.required_text, 1));
    raise(&mut capacity.rows[6], rows(parties.rows, 1));
    raise(&mut capacity.party_countries, rows(parties.rows, 1));
    raise(&mut capacity.party_text, rows(parties.bytes, 1));
    raise(&mut capacity.rows[10], rows(modifiers.rows, 1));
    raise(&mut capacity.modifier_text, rows(modifiers.bytes, 1));
    raise(&mut capacity.modifier_values, rows(modifiers.values, 1));
    match cdecision_status::random_item_demand(memory, base, state) {
        Ok(random) => {
            raise(&mut capacity.rows[24], rows(random.rows, 1));
            raise(&mut capacity.random_item_text, rows(random.bytes, 1));
        }
        Err(reason) => {
            refusal.get_or_insert(reason);
        }
    }
    refusal.map_or(Ok(()), Err)
}

fn add_rows(total: &mut usize, outcome: engine_native::walk::Outcome) -> Result<(), &'static str> {
    let count = match outcome {
        engine_native::walk::Outcome::Rows { len } => len,
        engine_native::walk::Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("governance sizing row count overflow")?,
        engine_native::walk::Outcome::Unavailable { reason } => return Err(reason),
    };
    *total = total
        .checked_add(count)
        .ok_or("governance sizing row count overflow")?;
    Ok(())
}

#[cfg(test)]
mod tests;
