use super::*;

fn increments(
    previous: &MarketPool,
    current: &MarketPool,
    event: MarketEventRow,
    kind: u8,
    output: &mut Vec<MarketEventRow>,
) {
    let mut values = BTreeMap::<u64, (i64, i64)>::new();
    for item in &previous.items {
        values.entry(item.variant).or_default().0 = item.amount_raw;
    }
    for item in &current.items {
        values.entry(item.variant).or_default().1 = item.amount_raw;
    }
    for (variant, (before, total)) in values {
        let amount_raw = total.wrapping_sub(before);
        if amount_raw != 0 {
            output.push(MarketEventRow {
                kind,
                variant,
                amount_raw,
                total_raw: total,
                lost_total_raw: if kind == LOST { total } else { 0 },
                mode: 0,
                ordinal: 0,
                auxiliary_raw: 0,
                date: 0,
                flags: DERIVED_TOTAL,
                ..event
            });
        }
    }
}
pub(super) fn delivery_unavailable(
    state: &mut State,
    event: MarketEventRow,
    contract: bool,
    demand: u8,
    reason: u32,
    output: &mut Vec<MarketEventRow>,
) {
    if contract {
        if demand & READ_PURCHASE_TOTAL == 0 {
            return;
        }
        state
            .unavailable_contracts
            .insert((event.country, event.agreement, 1));
    } else {
        if demand & (READ_DELIVERED | READ_SUNK) == 0 {
            return;
        }
        state
            .unavailable_agreements
            .insert((event.country, event.agreement));
    }
    for (bit, mode) in [
        (READ_DELIVERED, DELIVERED),
        (READ_SUNK, LOST),
        (READ_PURCHASE_TOTAL, CONTRACT_DELIVERED),
    ] {
        if contract != (mode == CONTRACT_DELIVERED) || demand & bit == 0 {
            continue;
        }
        DELIVERY_AMOUNT_REFUSALS.fetch_add(1, Ordering::Relaxed);
        output.push(MarketEventRow {
            kind: DELIVERY_UNAVAILABLE,
            mode,
            ordinal: reason,
            amount_raw: 0,
            auxiliary_raw: 0,
            total_raw: 0,
            lost_total_raw: 0,
            variant: 0,
            flags: 0,
            ..event
        });
    }
}
fn agreement_totals(
    state: &mut State,
    event: MarketEventRow,
    totals: [MarketPool; 2],
    output: &mut Vec<MarketEventRow>,
) {
    let key = (event.country, event.agreement);
    state.dirty_agreements.remove(&key);
    if state.unavailable_agreements.contains(&key) {
        delivery_unavailable(state, event, false, READ_DELIVERED | READ_SUNK, 2, output);
        return;
    }
    let held = state.agreements.entry(key).or_insert_with(|| Agreement {
        cutoff: 0,
        row: LendLeaseAgreementRow {
            country: key.0,
            id: key.1,
            receiver: event.receiver,
            ..Default::default()
        },
    });
    let [delivered, sunk] = totals;
    increments(&held.row.delivered, &delivered, event, DELIVERED, output);
    increments(&held.row.sunk, &sunk, event, LOST, output);
    held.row.delivered = delivered.into();
    held.row.sunk = sunk.into();
    state.changed_agreements.insert(key);
}
fn contract_total(equipment: &MarketPool, progress: i64, cost: i64) -> MarketPool {
    use replay_data::fixed_point::{fixed5_to_q15, q15_to_fixed5};
    let progress = fixed5_to_q15(progress);
    let cost = fixed5_to_q15(cost);
    let mut items = Vec::with_capacity(equipment.items.len());
    for item in &equipment.items {
        if item.amount_raw == 0 && progress != cost {
            continue;
        }
        let amount = if progress == cost {
            item.amount_raw
        } else if cost == 0 {
            0
        } else {
            let product = fixed5_to_q15(item.amount_raw).wrapping_mul(progress) & !0x7fff;
            q15_to_fixed5((i128::from(product) / i128::from(cost)) as i64)
        };
        if amount == 0 && progress != cost {
            continue;
        }
        items.push(MarketEquipment {
            variant: item.variant,
            amount_raw: i64::from((amount / 100_000) as i32) * 100_000,
        });
    }
    MarketPool {
        allow_zero: progress == cost && equipment.allow_zero,
        items,
    }
}
pub(super) fn read_cost(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    row: &MarketContractTermsRow,
) -> Result<i64, &'static str> {
    let definition = pointer + market::DRAFT;
    if scalar::<u8>(memory, definition, market::DEFINITION_COST_VALID)? != 0 {
        return scalar(memory, definition, market::DEFINITION_TOTAL_COST);
    }
    const GROUP_STRIDE: usize = 24;
    const GROUP_TOTAL: usize = 16;
    let groups =
        engine_native::cequipment_variant::pool::allocation_entries(memory, base, definition)?;
    let mut quoted = false;
    for index in 0..groups.count {
        if scalar::<i64>(
            memory,
            groups
                .element(index, GROUP_STRIDE)
                .ok_or("market quote group address overflow")?,
            GROUP_TOTAL,
        )? != 0
        {
            quoted = true;
            break;
        }
    }
    let levels: BTreeMap<_, _> = row
        .price_levels
        .iter()
        .map(|level| (level.variant, level.level))
        .collect();
    let factor: i64 = global(memory, base, market::MARKET_PRICE_FACTOR_RVA)?;
    let items = engine_native::cequipment_variant::pool::entries(
        memory,
        base,
        definition + market::DRAFT_EQUIPMENT,
    )?;
    let mut total = 0_i64;
    for index in 0..items.count {
        let item = engine_native_parts::equipment_entry::read(
            memory,
            base,
            items
                .element(index, engine_native_parts::equipment_entry::STRIDE)
                .ok_or("market cost item address overflow")?,
            index,
        )?;
        if item.amount_raw <= 0 {
            continue;
        }
        let quote = if quoted {
            match market::selected::item(memory, base, definition, item.variant_pointer) {
                Ok(quote) => quote.amount_raw,
                Err(
                    "market selected group absent"
                    | "market selected group differs"
                    | "market selected destination variant absent"
                    | "market selected destination differs",
                ) => 0,
                Err(reason) => return Err(reason),
            }
        } else {
            scalar(memory, item.variant_pointer, market::VARIANT_MARKET_PRICE)?
        };
        let level = match levels.get(&item.variant_handle).copied().unwrap_or(1) {
            0 => global(memory, base, market::LOW_PRICE_FACTOR_RVA)?,
            1 => 100_000_i64,
            2 => global(memory, base, market::HIGH_PRICE_FACTOR_RVA)?,
            _ => return Err("market cost price level unsupported"),
        };
        let price = (quote.wrapping_mul(level) / 100_000).wrapping_mul(factor) / 100_000;
        total = total.wrapping_add(item.amount_raw.wrapping_mul(price) / 100_000);
    }
    Ok(total)
}
pub(super) fn read_contract_total(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    row: &MarketContractTermsRow,
) -> Result<(MarketPool, i64), &'static str> {
    if row.kind == 0 {
        return Ok((MarketPool::default(), 0));
    }
    let progress = scalar(
        memory,
        pointer,
        engine_production::cpurchase_contract::EQUIPMENT_PROGRESS,
    )?;
    let completed: i32 = scalar(
        memory,
        pointer,
        engine_production::cpurchase_contract::COMPLETED,
    )?;
    if progress == 0 && completed == 0 {
        return Ok((
            MarketPool::default(),
            read_cost(memory, base, pointer, row)?,
        ));
    }
    let cost = read_cost(memory, base, pointer, row)?;
    Ok((contract_total(&row.equipment, progress, cost), cost))
}
fn finish_contract(
    state: &mut State,
    event: MarketEventRow,
    progress: i64,
    cost: i64,
    output: &mut Vec<MarketEventRow>,
) {
    let key = (event.country, event.agreement, 1);
    state.dirty_contracts.remove(&key);
    if state.unavailable_contracts.contains(&key)
        || !state.contracts.contains_key(&key) && (progress != 0 || event.ordinal != 0)
    {
        let reason = if state.unavailable_contracts.contains(&key) {
            2
        } else {
            4
        };
        delivery_unavailable(state, event, true, READ_PURCHASE_TOTAL, reason, output);
        return;
    }
    if let Some(held) = state
        .contracts
        .get_mut(&(event.country, event.agreement, 1))
    {
        if event.occurrence < held.cutoff {
            return;
        }
        let next = if progress == 0 && event.ordinal == 0 {
            MarketPool::default()
        } else {
            contract_total(
                &held.row.equipment,
                progress,
                if event.flags & 1 != 0 {
                    cost
                } else {
                    held.cost
                },
            )
        };
        increments(&held.delivered, &next, event, CONTRACT_DELIVERED, output);
        held.delivered = next;
    }
}
pub(super) fn erased_marks(
    state: &mut State,
    event: &MarketEventRow,
    output: &mut Vec<MarketEventRow>,
) {
    if matches!(event.kind, AMENDMENT | REMOVED) {
        let keys = [(event.country, event.agreement)];
        for key in keys {
            if let Some((mark, _, demand)) = state.dirty_agreements.remove(&key) {
                delivery_unavailable(state, mark, false, demand, 5, output);
            }
        }
    }
    if event.kind == CONTRACT_REMOVED && event.flags == 1 {
        let keys = [(event.country, event.agreement, 1)];
        for key in keys {
            if let Some((mark, _, demand)) = state.dirty_contracts.remove(&key) {
                delivery_unavailable(state, mark, true, demand, 5, output);
            }
        }
    }
}
pub(super) fn observe(
    state: &mut State,
    event: &MarketEventRow,
    output: &mut Vec<MarketEventRow>,
) -> Result<(), &'static str> {
    let key = (event.country, event.agreement);
    if event.kind == AMENDMENT_COST && event.mode == COUNTRY_RESET {
        reset_country(state, event.country);
        return Ok(());
    }
    if event.kind == AMENDMENT_COST && event.mode == ADMISSION {
        state
            .admissions
            .insert((key.0, key.1, event.flags), event.total_raw as usize);
        return Ok(());
    }
    if event.kind == AMENDMENT_COST && event.mode == HOLDER_MARK {
        let pointer = event.total_raw as usize;
        match event.flags {
            INVENTORY_MARK => {
                if state
                    .countries
                    .get(&event.country)
                    .is_some_and(|held| event.occurrence >= held.cutoff)
                {
                    state.dirty_inventory.insert(event.country);
                }
            }
            CONTRACT_MARK => {
                if state
                    .contracts
                    .get(&(key.0, key.1, 1))
                    .is_none_or(|held| event.occurrence >= held.cutoff)
                {
                    let entry = state
                        .dirty_contracts
                        .entry((key.0, key.1, 1))
                        .or_insert((*event, pointer, 0));
                    entry.0 = *event;
                    entry.1 = pointer;
                    entry.2 |= event.ordinal as u8;
                }
            }
            AGREEMENT_MARK => {
                if state
                    .agreements
                    .get(&key)
                    .is_none_or(|held| event.occurrence >= held.cutoff)
                {
                    let entry = state
                        .dirty_agreements
                        .entry(key)
                        .or_insert((*event, pointer, 0));
                    entry.0 = *event;
                    entry.1 = pointer;
                    entry.2 |= event.ordinal as u8;
                }
            }
            _ => return Err("market holder mark domain unsupported"),
        }
        return Ok(());
    }
    if event.kind == AMENDMENT_COST && event.mode == FINAL_CONTRACT {
        finish_contract(state, *event, event.amount_raw, event.auxiliary_raw, output);
        return Ok(());
    }
    if state
        .agreements
        .get(&key)
        .is_some_and(|held| event.occurrence < held.cutoff)
    {
        return Ok(());
    }
    let snapshot = (key.0, key.1, event.occurrence);
    match event.kind {
        FINAL_BEGIN => {
            state.terminal.insert(
                snapshot,
                [
                    MarketPool {
                        allow_zero: event.flags & 1 != 0,
                        items: Vec::new(),
                    },
                    MarketPool {
                        allow_zero: event.flags & 2 != 0,
                        items: Vec::new(),
                    },
                ],
            );
        }
        FINAL_ITEM => {
            let pools = state
                .terminal
                .get_mut(&snapshot)
                .ok_or("market final item without header")?;
            let pool = pools
                .get_mut(event.mode as usize)
                .ok_or("market final pool mode unsupported")?;
            if pool.items.len() != event.ordinal as usize {
                return Err("market final pool order incomplete");
            }
            pool.items.push(MarketEquipment {
                variant: event.variant,
                amount_raw: event.amount_raw,
            });
        }
        FINAL_END => {
            let pools = state
                .terminal
                .remove(&snapshot)
                .ok_or("market final totals without header")?;
            agreement_totals(state, *event, pools, output);
        }
        _ => return Err("market worker occurrence unsupported"),
    }
    Ok(())
}
pub(super) fn acquire_marked(
    state: &mut State,
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<MarketEventRow>,
) {
    for owner in std::mem::take(&mut state.dirty_inventory) {
        if state.pending_countries.contains_key(&owner) {
            continue;
        }
        let result = (|| {
            let held = state
                .countries
                .get_mut(&owner)
                .ok_or("market inventory holder absent")?;
            authenticate_country(memory, base, held.identity)?;
            market::country(
                memory,
                base,
                held.identity,
                held.manager,
                market::Demand {
                    stockpile: true,
                    ..Default::default()
                },
                &mut held.scratch,
            )?;
            held.row.inventory = pool(
                held.scratch.stockpile.allow_zero,
                held.scratch
                    .stockpile
                    .equipment
                    .iter()
                    .map(|row| (row.variant, row.amount_raw)),
            )
            .into();
            state.changed[0].insert(owner);
            Ok(())
        })();
        if let Err(reason) = result {
            refuse_country(state, owner, reason);
        }
    }
    for (key, (event, pointer, demand)) in std::mem::take(&mut state.dirty_agreements) {
        if state.refusals[2].is_some() {
            delivery_unavailable(state, event, false, demand, 2, output);
            continue;
        }
        let mut missing_demand = demand & (READ_DELIVERED | READ_SUNK);
        let result = (|| {
            let identity = lease::identity(memory, base, pointer)?;
            if (identity.giver, identity.id) != key {
                return Err("market marked agreement identity changed");
            }
            let held = state
                .agreements
                .get_mut(&key)
                .ok_or("market marked agreement has no amendment or seed")?;
            let mut failure = None;
            for (bit, offset, kind) in [
                (READ_DELIVERED, lease::DELIVERED, DELIVERED),
                (READ_SUNK, lease::SUNK, LOST),
            ] {
                if demand & bit == 0 {
                    continue;
                }
                match read_pool(memory, base, pointer + offset) {
                    Ok(next) => {
                        let previous = if kind == DELIVERED {
                            &mut held.row.delivered
                        } else {
                            &mut held.row.sunk
                        };
                        increments(previous, &next, event, kind, output);
                        *previous = next.into();
                        missing_demand &= !bit;
                    }
                    Err(reason) => {
                        failure.get_or_insert(reason);
                    }
                }
            }
            if demand & READ_ONCE != 0 {
                match read_pool(memory, base, pointer + lease::ONCE_NEED) {
                    Ok(next) => held.row.once = next.into(),
                    Err(reason) => {
                        failure.get_or_insert(reason);
                    }
                }
            }
            if demand & READ_DELIVERED != 0 {
                match scalar(memory, pointer, lease::LAST_DELIVERY_DATE) {
                    Ok(date) => held.row.last_delivery = date,
                    Err(reason) => {
                        failure.get_or_insert(reason);
                    }
                }
            }
            state.changed_agreements.insert(key);
            failure.map_or(Ok(()), Err)
        })();
        if let Err(reason) = result {
            state.refusals[2] = Some(reason);
            delivery_unavailable(state, event, false, missing_demand, 1, output);
        }
    }
    for (key, (event, pointer, demand)) in std::mem::take(&mut state.dirty_contracts) {
        if state.refusals[1].is_some() {
            delivery_unavailable(state, event, true, demand, 2, output);
            continue;
        }
        let mut amounts_complete = demand & READ_PURCHASE_TOTAL == 0;
        let result = (|| {
            let held = state
                .contracts
                .get_mut(&key)
                .ok_or("market marked contract not admitted")?;
            if read::<u64>(
                memory,
                pointer + engine_production::cpurchase_contract::ID_TYPE,
            ) != Some(held.row.id)
                || engine_sites::at(base, engine_production::cpurchase_contract::VTABLE_RVA)
                    .is_none_or(|vtable| read::<u64>(memory, pointer) != Some(vtable as u64))
            {
                return Err("market marked contract identity changed");
            }
            if demand & READ_PURCHASE_TOTAL != 0 {
                let (next, cost) = read_contract_total(memory, base, pointer, &held.row)?;
                increments(&held.delivered, &next, event, CONTRACT_DELIVERED, output);
                held.delivered = next;
                held.cost = cost;
                amounts_complete = true;
            }
            if event.occurrence >= held.cutoff {
                if demand & READ_RUNTIME != 0 {
                    let carried = held.row.runtime.route_equipment.clone();
                    held.row.runtime = read_runtime(memory, pointer, carried)?;
                }
                if demand & READ_ROUTE != 0 {
                    held.row.runtime.route_equipment =
                        read_pool(memory, base, pointer + market::CONTRACT_ROUTE_POOL)?.into();
                }
            }
            state.changed_contracts.insert(key);
            Ok(())
        })();
        if let Err(reason) = result {
            state.refusals[1] = Some(reason);
            if !amounts_complete {
                delivery_unavailable(state, event, true, demand, 1, output);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scalar_contract_tick_carries_the_large_route_pool_without_reading_or_cloning_it() {
        const POINTER: usize = 0x100;
        let id = (17_u64 << 32) | u64::from(engine_production::cpurchase_contract::TYPE);
        let mut bytes = vec![0; 0x600];
        bytes[POINTER..POINTER + 8].copy_from_slice(
            &(engine_production::cpurchase_contract::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        bytes[POINTER + engine_production::cpurchase_contract::ID_TYPE
            ..POINTER + engine_production::cpurchase_contract::ID_TYPE + 8]
            .copy_from_slice(&id.to_le_bytes());
        bytes[POINTER + market::CONTRACT_PERIOD_DAYS..POINTER + market::CONTRACT_PERIOD_DAYS + 4]
            .copy_from_slice(&17_i32.to_le_bytes());
        struct Scalars<'a>(&'a [u8]);
        impl Memory for Scalars<'_> {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                assert!(
                    !(POINTER + market::CONTRACT_ROUTE_POOL
                        ..POINTER + market::CONTRACT_ROUTE_LENGTH)
                        .contains(&at),
                    "scalar tick read the route pool"
                );
                assert_ne!(
                    at,
                    POINTER + engine_production::cpurchase_contract::EQUIPMENT_PROGRESS,
                    "scalar tick read delivery progress"
                );
                engine_native_memory::memory::SliceMemory {
                    base: 0,
                    bytes: self.0,
                }
                .copy(at, output)
            }
        }
        let route = MarketPool {
            items: vec![
                MarketEquipment {
                    variant: 31,
                    amount_raw: 99
                };
                5000
            ],
            ..Default::default()
        };
        let allocation = route.items.as_ptr();
        let terms = route.clone();
        let term_allocation = terms.items.as_ptr();
        let mut state = State::new(0);
        state.contracts.insert(
            (5000, id, 1),
            Contract {
                pointer: POINTER,
                cutoff: 0,
                delivered: MarketPool::default(),
                cost: 0,
                row: MarketContractTermsRow {
                    country: 5000,
                    id,
                    kind: 1,
                    runtime: MarketContractRuntime {
                        route_equipment: route.into(),
                        ..Default::default()
                    },
                    equipment: terms.into(),
                    ..Default::default()
                },
            },
        );
        let baseline = crate::test_support::with_allocations_denied(|| {
            replay_data::rows::RowCodec::try_clone(&state.contracts[&(5000, id, 1)].row).unwrap()
        });
        assert_eq!(baseline.runtime.route_equipment.items.as_ptr(), allocation);
        let sibling = (5000, id + 1, 1);
        let mut sibling_row = state.contracts[&(5000, id, 1)].row.clone();
        sibling_row.id += 1;
        let sibling_allocation = sibling_row.runtime.route_equipment.items.as_ptr();
        state.contracts.insert(
            sibling,
            Contract {
                pointer: usize::MAX,
                cutoff: 0,
                row: sibling_row,
                delivered: MarketPool::default(),
                cost: 0,
            },
        );
        let event = MarketEventRow {
            country: 5000,
            agreement: id,
            occurrence: 1,
            kind: AMENDMENT_COST,
            mode: HOLDER_MARK,
            flags: CONTRACT_MARK,
            ordinal: u32::from(READ_RUNTIME),
            total_raw: POINTER as i64,
            ..Default::default()
        };
        let mut output = Vec::new();
        observe(&mut state, &event, &mut output).unwrap();
        acquire_marked(&mut state, &Scalars(&bytes), 0, &mut output);
        let runtime = &state.contracts[&(5000, id, 1)].row.runtime;
        assert_eq!(state.refusals, [None; 3]);
        assert_eq!(runtime.period_days, 17);
        assert_eq!(runtime.route_equipment.items.as_ptr(), allocation);
        assert!(output.is_empty());
        let changes = publish_changes(&mut state, false);
        assert_eq!(
            changes.contracts.len(),
            1,
            "scalar tick published an untouched sibling"
        );
        assert_eq!(
            changes.contract_keys,
            [(5000_u128 << 72) | (u128::from(id) << 8) | 1]
        );
        assert_eq!(
            changes.contracts[0].runtime.route_equipment.items.as_ptr(),
            allocation,
            "publication cloned the carried route pool"
        );
        assert_eq!(
            changes.contracts[0].equipment.items.as_ptr(),
            term_allocation,
            "publication cloned immutable terms"
        );
        assert_eq!(
            state.contracts[&sibling]
                .row
                .runtime
                .route_equipment
                .items
                .as_ptr(),
            sibling_allocation
        );
    }
    #[test]
    fn failed_delivery_reads_and_subsequent_marks_name_the_missing_holder_amounts() {
        struct Missing;
        impl Memory for Missing {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        let mut state = State::new(0);
        let mut output = Vec::new();
        for occurrence in [1, 2] {
            for (contract, id, demand) in [
                (false, 17, READ_DELIVERED | READ_SUNK),
                (true, 18, READ_PURCHASE_TOTAL),
            ] {
                let event = MarketEventRow {
                    occurrence,
                    country: 5000,
                    receiver: 4999,
                    agreement: id,
                    ordinal: u32::from(demand),
                    flags: if contract {
                        CONTRACT_MARK
                    } else {
                        AGREEMENT_MARK
                    },
                    mode: HOLDER_MARK,
                    kind: AMENDMENT_COST,
                    total_raw: -1,
                    ..Default::default()
                };
                observe(&mut state, &event, &mut output).unwrap();
            }
            acquire_marked(&mut state, &Missing, 0, &mut output);
        }
        assert_eq!(output.len(), 6);
        assert!(
            output.iter().all(|row| row.kind == DELIVERY_UNAVAILABLE
                && row.delivery_unavailable_reason().is_some())
        );
        assert_eq!(output.iter().filter(|row| row.agreement == 17).count(), 4);
        assert_eq!(output.iter().filter(|row| row.agreement == 18).count(), 2);
        assert!(
            output
                .iter()
                .filter(|row| row.occurrence == 2)
                .all(|row| row.ordinal == 2)
        );
        let terminal = MarketEventRow {
            country: 5000,
            agreement: 18,
            occurrence: 3,
            flags: 1,
            ..Default::default()
        };
        finish_contract(&mut state, terminal, 50_000, 100_000, &mut output);
        assert_eq!(output.last().unwrap().kind, DELIVERY_UNAVAILABLE);
        assert_eq!(output.last().unwrap().agreement, 18);
    }
    #[test]
    fn invalid_lazy_contract_cost_uses_native_quotes_and_live_mod_factors_without_writing_game_memory()
     {
        use engine_native::{
            ccountry::synthetic::World,
            cequipment_variant::{self as variant, pool as native_pool},
        };
        let mut world = World::new(0, 5000, 0x20);
        let pointer = world.alloc(0x400);
        let definition = pointer + market::DRAFT;
        let model = world.alloc(0x400);
        world.u64(model, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
        let reference = (31_u64 << 32) | variant::REFERENCE_TYPE as u64;
        world.u64(model + 8, reference);
        world.u64(model + market::VARIANT_MARKET_PRICE, 200_000);
        world.u64(definition, native_pool::VTABLE_RVA.0 as u64);
        world.u64(
            definition + market::DRAFT_EQUIPMENT,
            native_pool::VTABLE_RVA.0 as u64,
        );
        let items = world.alloc(16);
        world.u64(items, model as u64);
        world.u64(items + 8, 300_000);
        world.array(
            definition + market::DRAFT_EQUIPMENT + native_pool::ITEMS,
            items,
            1,
        );
        struct Native<'a>(&'a World);
        impl Memory for Native<'_> {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                if at == market::LOW_PRICE_FACTOR_RVA.0 {
                    output.copy_from_slice(&50_000_i64.to_le_bytes());
                    true
                } else if at == market::MARKET_PRICE_FACTOR_RVA.0 {
                    output.copy_from_slice(&150_000_i64.to_le_bytes());
                    true
                } else {
                    self.0.memory().copy(at, output)
                }
            }
        }
        let row = MarketContractTermsRow {
            kind: 1,
            price_levels: vec![MarketPriceLevel {
                variant: reference,
                level: 0,
            }]
            .into(),
            ..Default::default()
        };
        assert_eq!(
            read_cost(&Native(&world), 0, pointer, &row).unwrap(),
            450_000
        );
        let accepted = MarketContractTermsRow {
            kind: 1,
            ..row.clone()
        };
        let (delivered, cost) =
            read_contract_total(&Native(&world), 0, pointer, &accepted).unwrap();
        assert!(delivered.items.is_empty());
        assert_eq!(
            cost, 450_000,
            "zero-progress admission lost the immutable terminal cost baseline"
        );
        world.u32(definition + market::DEFINITION_COST_VALID, 1);
        world.u64(definition + market::DEFINITION_TOTAL_COST, 900_000);
        struct Cached<'a> {
            world: &'a World,
            definition: usize,
        }
        impl Memory for Cached<'_> {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                assert!(
                    [
                        self.definition + market::DEFINITION_COST_VALID,
                        self.definition + market::DEFINITION_TOTAL_COST
                    ]
                    .contains(&at)
                );
                self.world.memory().copy(at, output)
            }
        }
        assert_eq!(
            read_cost(
                &Cached {
                    world: &world,
                    definition
                },
                0,
                pointer,
                &row
            )
            .unwrap(),
            900_000
        );
    }
    #[test]
    fn purchased_whole_unit_totals_difference_without_a_native_operand_copy() {
        let equipment = MarketPool {
            allow_zero: true,
            items: vec![
                MarketEquipment {
                    variant: 31,
                    amount_raw: 300_000,
                },
                MarketEquipment {
                    variant: 32,
                    amount_raw: 700_000,
                },
            ],
        };
        let half = contract_total(&equipment, 500_000, 1_000_000);
        assert_eq!(
            half.items
                .iter()
                .map(|row| row.amount_raw)
                .collect::<Vec<_>>(),
            [100_000, 300_000]
        );
        let all = contract_total(&equipment, 1_000_000, 1_000_000);
        assert_eq!(all, equipment);
        let mut increments_out = Vec::new();
        increments(
            &half,
            &all,
            MarketEventRow::default(),
            CONTRACT_DELIVERED,
            &mut increments_out,
        );
        assert_eq!(
            increments_out
                .iter()
                .map(|row| row.amount_raw)
                .collect::<Vec<_>>(),
            [200_000, 400_000]
        );
    }
    #[test]
    fn marked_agreement_reads_only_the_selected_pool_and_never_the_source_or_other_schedules() {
        use engine_native::{
            ccountry::synthetic::World,
            cequipment_variant::{self as variant, pool as native_pool},
        };
        let mut world = World::new(0, 5000, 0x20);
        let resources = world.alloc(0x30);
        world.u64(
            resources,
            engine_resources::ccountry_resources::VTABLE_RVA.0 as u64,
        );
        world.u32(
            resources + engine_resources::ccountry_resources::OWNER,
            5000,
        );
        let pointer = world.alloc(lease::SIZE);
        let id = (17_u64 << 32) | 9001;
        world.u64(pointer, lease::VTABLE_RVA.0 as u64);
        world.u64(pointer + lease::HANDLE, id);
        world.u64(pointer + lease::GIVER_RESOURCES, resources as u64);
        world.u32(pointer + lease::RECEIVER, 4999);
        world.u64(pointer + lease::DELIVERED, native_pool::VTABLE_RVA.0 as u64);
        let entries = world.alloc(5000 * 16);
        for index in 0..5000 {
            let model = world.alloc(16);
            world.u64(model, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
            world.u64(
                model + 8,
                ((index as u64 + 1) << 32) | variant::REFERENCE_TYPE as u64,
            );
            world.u64(entries + index * 16, model as u64);
            world.u64(entries + index * 16 + 8, 17);
        }
        world.array(
            pointer + lease::DELIVERED + native_pool::ITEMS,
            entries,
            5000,
        );
        struct Selected<'a> {
            world: &'a World,
            pointer: usize,
        }
        impl Memory for Selected<'_> {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                for pool in [
                    lease::MONTHLY_NEED,
                    lease::PERCENTAGE_NEED,
                    lease::ONCE_NEED,
                    lease::SUNK,
                ] {
                    assert!(
                        !(self.pointer + pool..self.pointer + pool + 0x40).contains(&at),
                        "unselected native pool read"
                    );
                }
                self.world.memory().copy(at, output)
            }
        }
        let event = MarketEventRow {
            country: 5000,
            agreement: id,
            receiver: 4999,
            occurrence: 1,
            kind: AMENDMENT_COST,
            mode: HOLDER_MARK,
            flags: AGREEMENT_MARK,
            ordinal: u32::from(READ_DELIVERED),
            total_raw: pointer as i64,
            ..Default::default()
        };
        let mut state = State::new(0);
        state.agreements.insert(
            (5000, id),
            Agreement {
                cutoff: 0,
                row: LendLeaseAgreementRow {
                    country: 5000,
                    id,
                    ..Default::default()
                },
            },
        );
        let mut output = Vec::new();
        observe(&mut state, &event, &mut output).unwrap();
        acquire_marked(
            &mut state,
            &Selected {
                world: &world,
                pointer,
            },
            0,
            &mut output,
        );
        assert_eq!(state.refusals, [None; 3]);
        assert_eq!(output.len(), 5000);
        assert_eq!(
            state.agreements[&(5000, id)].row.delivered.items.len(),
            5000
        );
        assert!(state.dirty_agreements.is_empty());
    }
}
