use super::*;

pub(super) fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_inner(memory, base, game_state, engine_native_memory::walk::Buffer::Growing(output))
        .unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Result<Outcome, &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut found_country = false;
    let mut len = 0_usize;
    let mut unread_rows = 0_u32;
    let mut seen = engine_native_memory::slot_set::SlotSet::new();
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        found_country = true;
        let agency_pointer = pointer(memory, country.pointer, ccountry::INTELLIGENCE_AGENCY)?;
        if agency_pointer == 0 {
            continue;
        }
        authenticate(memory, base, agency_pointer, agency::VTABLE_RVA)?;
        if pointer(memory, agency_pointer, agency::OWNER_COUNTRY)? != country.pointer {
            return Err("cryptology agency owner mismatch");
        }
        let crypto = pointer(memory, agency_pointer, agency::CRYPTOLOGY)?;
        if crypto == 0 {
            continue;
        }
        authenticate(memory, base, crypto, VTABLE_RVA)?;
        if pointer(memory, crypto, OWNER_COUNTRY)? != country.pointer {
            return Err("cryptology owner mismatch");
        }
        let states = crypto
            .checked_add(DECRYPTION_STATES)
            .and_then(|address| Array::read(memory, address, state::STRIDE))
            .ok_or("cryptology state array unreadable or invalid")?;
        output.ensure(
            len.checked_add(states.count as usize)
                .ok_or("cryptology row demand overflow")?,
        )?;
        let owner_start = len;
        for index in 0..states.count {
            if len == output.len() {
                unread_rows = unread_rows
                    .checked_add(states.count - index)
                    .ok_or("cryptology unread row count overflow")?;
                break;
            }
            let at = states
                .element(index, state::STRIDE)
                .ok_or("cryptology row address overflow")?;
            let mut row = read_state(memory, base, at)?;
            let target = countries.resolve(row.target_country)?;
            let slot = target.canonical_slot;
            let earlier = &output[owner_start..len];
            if seen.insert(slot, || {
                Ok::<_, &'static str>(earlier.iter().any(|p| p.owner_country == slot))
            })? {
                return Err("duplicate cryptology target");
            }
            row.owner_country = target.canonical_slot;
            row.ordinal = index;
            output[len] = row;
            len += 1;
        }
        seen.clear();
        for row in &mut output[owner_start..len] {
            row.owner_country = country.id;
        }
    }
    if !found_country {
        Err("country table has no readable countries")
    } else if unread_rows != 0 {
        Ok(Outcome::Truncated { len, unread_rows })
    } else {
        Ok(Outcome::Rows { len })
    }
}
