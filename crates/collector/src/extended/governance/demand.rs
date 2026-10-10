//! Off-thread capacity hints never supply recorded values or certify a read.

use super::{
    Capacities, Memory, ccountry, cdecision_status, cdynamic_modifier_container, cpolitical_party,
};
use crate::country_scale::{DemandFailures, raise, rows};

static FAILURES: [DemandFailures; 14] = [const { DemandFailures::new() }; 14];

pub(crate) fn status() -> impl serde::Serialize {
    crate::status::map(
        [
            "countries",
            "political_parties",
            "dynamic_modifiers",
            "decision_random_items",
            "focus",
            "research_slots",
            "party_leaders",
            "active_ideas",
            "timed_ideas",
            "decisions",
            "preferred_tactics",
            "technology_sharing",
            "power_balances",
            "research_inputs",
        ]
        .into_iter()
        .zip(FAILURES.iter().map(DemandFailures::diagnostics)),
    )
}

pub(super) fn presize(
    capacity: &mut Capacities,
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<(), &'static str> {
    let countries = FAILURES[0].observe(ccountry::Reader::new(memory, base, state))?;
    let mut parties = cpolitical_party::Demand::default();
    let mut modifiers = cdynamic_modifier_container::Demand::default();
    let mut refusal = None;
    capacity.raise_for_countries(countries.count() as usize);
    let mut focus_rows = [0_usize; 2];
    let mut focus_bytes = 0_usize;
    let mut slots = 0_usize;
    let mut leaders = 0_usize;
    let mut ideas = engine::cpolitical_status::IdeaDemand::default();
    let mut decisions = [0_usize; 2];
    let mut tactic_bytes = 0_usize;
    let mut research_rows = [0_usize; 3];
    let research_context = engine::research_inputs::Context::read(memory, base);
    let modifier_context = engine::research_modifiers::Context::read(memory, base);
    let mut research_inputs = Vec::new();
    for slot in 1..countries.count() {
        let country = match FAILURES[0].observe(countries.at(slot)) {
            Ok(Some(country)) => country,
            Ok(None) => continue,
            Err(reason) => {
                refusal.get_or_insert(reason);
                continue;
            }
        };
        let focus = engine::cnational_focus_progress::walk(
            memory,
            base,
            country.pointer,
            country.id,
            &mut [],
            &mut [],
            &mut [],
        );
        for (index, outcome) in [focus.outcome, focus.completed_outcome]
            .into_iter()
            .enumerate()
        {
            if let Err(reason) = FAILURES[4].observe(add_rows(&mut focus_rows[index], outcome)) {
                refusal.get_or_insert(reason);
            }
        }
        focus_bytes = focus_bytes
            .checked_add(focus.required_bytes)
            .ok_or("focus sizing byte count overflow")?;
        let slot_demand = engine::ctechnology_status::walk_slots(
            memory,
            base,
            country.pointer,
            country.id,
            &mut [],
        );
        if let Err(reason) = FAILURES[5].observe(add_rows(&mut slots, slot_demand)) {
            refusal.get_or_insert(reason);
        }
        match engine::cpolitical_status::Reader::new(memory, base, country.pointer) {
            Ok(reader) => {
                if let Err(reason) = FAILURES[6].observe(add_rows(
                    &mut leaders,
                    reader.party_leaders(country.id, &mut []),
                )) {
                    refusal.get_or_insert(reason);
                }
                let active = reader.ideas(&mut [], &mut []);
                if let Err(reason) = FAILURES[7].observe(add_rows(&mut ideas.rows, active.outcome))
                {
                    refusal.get_or_insert(reason);
                }
                ideas.bytes = ideas
                    .bytes
                    .checked_add(active.required_bytes)
                    .ok_or("active idea sizing byte count overflow")?;
            }
            Err(reason) => {
                for family in [6, 7] {
                    FAILURES[family].observe::<()>(Err(reason)).ok();
                }
                refusal.get_or_insert(reason);
            }
        }
        let decision = cdecision_status::walk(memory, base, country.pointer, &mut [], &mut []);
        if let Err(reason) = FAILURES[9].observe(add_rows(&mut decisions[0], decision.outcome)) {
            refusal.get_or_insert(reason);
        }
        decisions[1] = decisions[1]
            .checked_add(decision.required_bytes)
            .ok_or("decision sizing byte count overflow")?;
        let tactic =
            engine::ccombat_tactic::preferred(memory, base, country.pointer, country.id, &mut []);
        if let Err(reason) = FAILURES[10].observe(match tactic.outcome {
            engine::walk::Outcome::Unavailable { reason } => Err(reason),
            _ => Ok(()),
        }) {
            refusal.get_or_insert(reason);
        }
        tactic_bytes = tactic_bytes
            .checked_add(tactic.required_token_bytes)
            .ok_or("preferred tactic sizing byte count overflow")?;
        // Hints use fallible advisory storage: a failed hint cannot terminate
        // capture. 140EBDB60/140ECE4F0 supply the original slot population.
        let mut local_slots = 0;
        let prepared = add_rows(&mut local_slots, slot_demand).and_then(|()| {
            research_inputs
                .try_reserve(local_slots.saturating_sub(research_inputs.len()))
                .map_err(|_| "research sizing scratch allocation failed")?;
            research_inputs.resize_with(local_slots, engine::research_inputs::Row::default);
            Ok(())
        });
        let (input, bonuses) = match prepared {
            Ok(()) => engine::research_inputs::walk_country(
                memory,
                base,
                country.pointer,
                country.id,
                research_context,
                &mut research_inputs,
                &mut [],
            ),
            Err(reason) => (
                engine::walk::Outcome::Unavailable { reason },
                engine::walk::Outcome::Unavailable { reason },
            ),
        };
        for (index, outcome) in [input, bonuses].into_iter().enumerate() {
            if let Err(reason) = FAILURES[13].observe(add_rows(&mut research_rows[index], outcome))
            {
                refusal.get_or_insert(reason);
            }
        }
        if let engine::walk::Outcome::Rows { len } = input {
            let modifiers = engine::research_modifiers::walk_country(
                memory,
                base,
                &countries,
                country,
                modifier_context,
                &research_inputs[..len],
                &mut [],
            );
            if let Err(reason) = FAILURES[13].observe(add_rows(&mut research_rows[2], modifiers)) {
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
        if let Err(reason) = FAILURES[1].observe(party) {
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
        if let Err(reason) = FAILURES[2].observe(modifier) {
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
    match FAILURES[8].observe(engine::cpolitical_status::timed_idea_demand(
        memory, base, state,
    )) {
        Ok(timed) => {
            raise(&mut capacity.rows[23], rows(timed.rows, 1));
            raise(&mut capacity.timed_idea_text, rows(timed.bytes, 1));
        }
        Err(reason) => {
            refusal.get_or_insert(reason);
        }
    }
    let sharing = engine::ctechnology_sharing_group::walk(memory, base, state, &mut [], &mut []);
    for (index, outcome) in [(12, sharing.groups), (13, sharing.members)] {
        let mut count = 0;
        if let Err(reason) = FAILURES[11].observe(add_rows(&mut count, outcome)) {
            refusal.get_or_insert(reason);
        }
        raise(&mut capacity.rows[index], rows(count, 1));
    }
    let balances = engine::power_balance_manager::walk(
        memory,
        base,
        state,
        engine::power_balance_manager::Buffers {
            records: &mut [],
            members: &mut [],
            ranges: &mut [],
            overrides: &mut [],
            text: &mut [],
        },
    );
    for (index, outcome) in [
        (14, balances.records),
        (15, balances.members),
        (16, balances.ranges),
        (17, balances.overrides),
    ] {
        let mut count = 0;
        if let Err(reason) = FAILURES[12].observe(add_rows(&mut count, outcome)) {
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
    match FAILURES[3].observe(cdecision_status::random_item_demand(memory, base, state)) {
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

fn add_rows(total: &mut usize, outcome: engine::walk::Outcome) -> Result<(), &'static str> {
    let count = match outcome {
        engine::walk::Outcome::Rows { len } => len,
        engine::walk::Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("governance sizing row count overflow")?,
        engine::walk::Outcome::Unavailable { reason } => return Err(reason),
    };
    *total = total
        .checked_add(count)
        .ok_or("governance sizing row count overflow")?;
    Ok(())
}

#[cfg(test)]
mod tests;
