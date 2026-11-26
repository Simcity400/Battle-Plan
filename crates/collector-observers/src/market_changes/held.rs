use super::*;
use engine_native::ccountry;
use engine_native_memory::cpdx_array::Array;
use engine_production::market_ledger as market;
use engine_sites::Rva;
use replay_data::rows::{
    CountryMarketRow, LendLeaseAgreementRow, MarketContractRuntime, MarketContractTermsRow,
    MarketEquipment, MarketPool, MarketPriceLevel, MarketSubsidy, MarketVariable, SharedMarketPool,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};
const PENDING_REQUESTS: usize = 0x60;
const REQUEST_VTABLE_RVA: Rva = Rva(0x2aa1c88);
mod worker;

static EPOCH: AtomicU64 = AtomicU64::new(1);
static RECOVERY: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<State> = Mutex::new(State::new(0));
struct Country {
    identity: engine_native_memory::country_identity::Country,
    manager: usize,
    cutoff: u64,
    row: CountryMarketRow,
    scratch: market::Country,
}
struct Agreement {
    cutoff: u64,
    row: LendLeaseAgreementRow,
}
struct Contract {
    pointer: usize,
    cutoff: u64,
    row: MarketContractTermsRow,
    delivered: MarketPool,
    cost: i64,
}
struct State {
    epoch: u64,
    complete: bool,
    publish_full: bool,
    publish_countries_full: bool,
    country_manager: Option<usize>,
    uncertified_watches: [bool; 3],
    renewed_watches: [Option<usize>; 4],
    contracts_complete: bool,
    contracts_attempted: bool,
    discovered_contracts: BTreeSet<(u32, u64, u8)>,
    dirty_inventory: BTreeSet<u32>,
    dirty_agreements: BTreeMap<(u32, u64), (MarketEventRow, usize, u8)>,
    dirty_contracts: BTreeMap<(u32, u64, u8), (MarketEventRow, usize, u8)>,
    terminal: BTreeMap<(u32, u64, u64), [MarketPool; 2]>,
    variable_owners: BTreeMap<usize, (u32, u64, u8)>,
    admissions: BTreeMap<(u32, u64, u8), usize>,
    failure: Option<&'static str>,
    countries: BTreeMap<u32, Country>,
    contracts: BTreeMap<(u32, u64, u8), Contract>,
    agreements: BTreeMap<(u32, u64), Agreement>,
    changed: [BTreeSet<u32>; 2],
    changed_contracts: BTreeSet<(u32, u64, u8)>,
    pending_countries: BTreeMap<u32, (engine_native_memory::country_identity::Country, usize)>,
    unseeded_countries: BTreeSet<u32>,
    unavailable_agreements: BTreeSet<(u32, u64)>,
    unavailable_contracts: BTreeSet<(u32, u64, u8)>,
    changed_agreements: BTreeSet<(u32, u64)>,
    last_fuel: BTreeSet<(u32, u64)>,
    fuel_hour: Option<i32>,
    refusals: [Option<&'static str>; 3],
}
impl State {
    const fn new(epoch: u64) -> Self {
        Self {
            epoch,
            complete: false,
            publish_full: true,
            publish_countries_full: false,
            country_manager: None,
            uncertified_watches: [false; 3],
            renewed_watches: [None; 4],
            contracts_complete: false,
            contracts_attempted: false,
            discovered_contracts: BTreeSet::new(),
            dirty_inventory: BTreeSet::new(),
            dirty_agreements: BTreeMap::new(),
            dirty_contracts: BTreeMap::new(),
            terminal: BTreeMap::new(),
            variable_owners: BTreeMap::new(),
            admissions: BTreeMap::new(),
            failure: None,
            countries: BTreeMap::new(),
            contracts: BTreeMap::new(),
            agreements: BTreeMap::new(),
            changed: [const { BTreeSet::new() }; 2],
            changed_contracts: BTreeSet::new(),
            pending_countries: BTreeMap::new(),
            unseeded_countries: BTreeSet::new(),
            unavailable_agreements: BTreeSet::new(),
            unavailable_contracts: BTreeSet::new(),
            changed_agreements: BTreeSet::new(),
            last_fuel: BTreeSet::new(),
            fuel_hour: None,
            refusals: [None; 3],
        }
    }
    fn recertify_countries(&mut self) {
        for (&owner, held) in &self.countries {
            self.pending_countries
                .entry(owner)
                .or_insert((held.identity, held.manager));
        }
    }
    fn bind_country_manager(&mut self, manager: usize) {
        if self
            .country_manager
            .replace(manager)
            .is_some_and(|old| old != manager)
        {
            self.recertify_countries();
            for (_, pending_manager) in self.pending_countries.values_mut() {
                *pending_manager = manager;
            }
        }
    }
}
pub fn invalidate() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
    RECOVERY.store(false, Ordering::SeqCst);
}
pub(super) fn begin_hour() -> [Option<usize>; 4] {
    let mut state = STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if state.epoch != EPOCH.load(Ordering::Acquire) {
        return [None; 4];
    }
    if state.refusals[0].take().is_some() {
        state.publish_countries_full = true;
    }
    let demand = COUNTRY_WATCH_DEMAND.load(Ordering::Acquire);
    let mut renew = [None; 4];
    for (domain, watch) in WATCHES.iter().chain([&RESET_WATCH]).enumerate() {
        let missing = state.pending_countries.keys().any(|&owner| {
            let receiver = country_receiver(owner);
            watch.observed_sequence(receiver).is_none() && watch.register(receiver).is_none()
        });
        if !missing {
            state.renewed_watches[domain] = None;
        } else if watch.growth_pending()
            || state.renewed_watches[domain].is_none_or(|previous| demand > previous)
        {
            renew[domain] = Some(demand);
        }
    }
    renew
}
pub(super) fn finish_renewals(renewed: [Option<usize>; 4]) {
    if renewed.iter().all(Option::is_none) {
        return;
    }
    let mut state = STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (domain, demand) in renewed.into_iter().enumerate() {
        if demand.is_some() {
            let watch = WATCHES.get(domain).unwrap_or(&RESET_WATCH);
            state.renewed_watches[domain] = demand.filter(|_| !watch.growth_pending());
        }
    }
    state.recertify_countries();
}
pub(super) fn recovery_requested() -> bool {
    RECOVERY.load(Ordering::SeqCst)
}
pub fn full_publication_required() -> bool {
    let state = STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.epoch != EPOCH.load(Ordering::Acquire) || state.publish_full
}
fn pool(allow_zero: bool, items: impl IntoIterator<Item = (u64, i64)>) -> MarketPool {
    MarketPool {
        allow_zero,
        items: items
            .into_iter()
            .map(|(variant, amount_raw)| MarketEquipment {
                variant,
                amount_raw,
            })
            .collect(),
    }
}
fn levels(value: &[market::PriceLevel]) -> Vec<MarketPriceLevel> {
    value
        .iter()
        .map(|e| MarketPriceLevel {
            variant: e.variant,
            level: e.level,
        })
        .collect()
}
fn subsidies(value: &[market::Subsidy]) -> Vec<MarketSubsidy> {
    value
        .iter()
        .map(|e| MarketSubsidy {
            cic_raw: e.cic_raw,
            archetype: e.archetype,
            kind: e.kind,
            targets: e.targets.clone(),
            trigger: e.trigger.clone(),
        })
        .collect()
}
fn read_pool(memory: &impl Memory, base: usize, at: usize) -> Result<MarketPool, &'static str> {
    let mut output = market::Pool::default();
    market::pool(memory, base, at, &mut output)?;
    Ok(pool(
        output.allow_zero,
        output.equipment.iter().map(|e| (e.variant, e.amount_raw)),
    ))
}
fn read_agreement(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<LendLeaseAgreementRow, &'static str> {
    let identity = lease::identity(memory, base, pointer)?;
    let mut terms = lease::Terms::default();
    lease::terms(memory, base, pointer, identity, &mut terms)?;
    Ok(LendLeaseAgreementRow {
        country: identity.giver,
        id: identity.id,
        receiver: identity.receiver,
        active_since: terms.active_since,
        last_delivery: scalar(memory, pointer, lease::LAST_DELIVERY_DATE)?,
        monthly: pool(
            terms.monthly.allow_zero,
            terms
                .monthly
                .equipment
                .iter()
                .map(|e| (e.variant, e.amount_raw)),
        )
        .into(),
        percentage: pool(
            terms.percentage.allow_zero,
            terms
                .percentage
                .equipment
                .iter()
                .map(|e| (e.variant, e.amount_raw)),
        )
        .into(),
        once: pool(
            terms.once.allow_zero,
            terms
                .once
                .equipment
                .iter()
                .map(|e| (e.variant, e.amount_raw)),
        )
        .into(),
        delivered: read_pool(memory, base, pointer + lease::DELIVERED)?.into(),
        sunk: read_pool(memory, base, pointer + lease::SUNK)?.into(),
        fuel_daily_raw: terms.fuel_daily_raw,
        fuel_percentage_raw: terms.fuel_percentage_raw,
        fuel_sent_raw: scalar(memory, pointer, lease::FUEL_SENT)?,
        fuel_sunk_raw: scalar(memory, pointer, lease::FUEL_SUNK)?,
        last_fuel_delivered_raw: scalar(memory, pointer, lease::LAST_FUEL_DELIVERED)?,
        ..Default::default()
    })
}
fn read_contract(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    request: bool,
) -> Result<MarketContractTermsRow, &'static str> {
    let expected = engine_sites::at(
        base,
        if request {
            REQUEST_VTABLE_RVA
        } else {
            engine_production::cpurchase_contract::VTABLE_RVA
        },
    )
    .ok_or("market terms holder vtable mismatch")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("market terms holder vtable mismatch");
    }
    let id = scalar(
        memory,
        pointer,
        engine_production::cpurchase_contract::ID_TYPE,
    )?;
    let mut draft = market::Draft::default();
    let mut prices = market::Pool::default();
    if request {
        market::draft(memory, base, pointer + market::DRAFT, &mut draft)?;
        market::pool(memory, base, pointer + market::DRAFT, &mut prices)?;
    } else {
        market::contract_terms(memory, base, pointer, id, &mut draft, &mut prices)?;
    }
    Ok(MarketContractTermsRow {
        country: scalar(
            memory,
            pointer,
            engine_production::cpurchase_contract::SELLER,
        )?,
        id,
        kind: u8::from(!request),
        buyer: scalar(
            memory,
            pointer,
            engine_production::cpurchase_contract::BUYER,
        )?,
        speed: scalar(
            memory,
            pointer,
            engine_production::cpurchase_contract::SPEED,
        )?,
        equipment: pool(
            draft.equipment.allow_zero,
            draft
                .equipment
                .equipment
                .iter()
                .map(|e| (e.variant, e.amount_raw)),
        )
        .into(),
        prices: pool(
            prices.allow_zero,
            prices.equipment.iter().map(|e| (e.variant, e.amount_raw)),
        )
        .into(),
        price_levels: levels(&draft.price_levels).into(),
        subsidies: subsidies(&draft.subsidies).into(),
        runtime: if request {
            Default::default()
        } else {
            read_runtime(
                memory,
                pointer,
                read_pool(memory, base, pointer + market::CONTRACT_ROUTE_POOL)?.into(),
            )?
        },
        variables: (if request {
            Vec::new()
        } else {
            read_variables(memory, base, pointer + market::CONTRACT_VARIABLES)?
        })
        .into(),
        variable_state: if request {
            0
        } else {
            scalar(
                memory,
                pointer + market::CONTRACT_VARIABLES,
                engine_scripts::cvariables::SEED,
            )?
        },
    })
}
fn read_variables(
    memory: &impl Memory,
    base: usize,
    at: usize,
) -> Result<Vec<MarketVariable>, &'static str> {
    let mut rows = Vec::new();
    let mut names = Vec::new();
    let result = engine_scripts::cvariables::walk(memory, base, at, &mut rows, &mut names);
    let engine_native_memory::walk::Outcome::Rows { len } = result.outcome else {
        return Err("market contract variables unavailable");
    };
    let mut output: Vec<_> = rows[..len]
        .iter()
        .map(|row| MarketVariable {
            name: names[row.name_start..row.name_start + row.name_len].to_vec(),
            amount_raw: row.value_raw,
        })
        .collect();
    output.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(output)
}
fn read_runtime(
    memory: &impl Memory,
    at: usize,
    route_equipment: SharedMarketPool,
) -> Result<MarketContractRuntime, &'static str> {
    Ok(MarketContractRuntime {
        route_equipment,
        route_length: scalar(memory, at, market::CONTRACT_ROUTE_LENGTH)?,
        route_type: scalar(memory, at, market::CONTRACT_ROUTE_TYPE)?,
        route_request: scalar(memory, at, market::CONTRACT_ROUTE_REQUEST)?,
        route_sunk_convoys_raw: scalar(memory, at, market::CONTRACT_ROUTE_LOSSES)?,
        period_days: scalar(memory, at, market::CONTRACT_PERIOD_DAYS)?,
        meta_cic_raw: scalar(memory, at, market::CONTRACT_META_CIC)?,
        meta_convoys_raw: scalar(memory, at, market::CONTRACT_META_CONVOYS)?,
        meta_efficiency_raw: scalar(memory, at, market::CONTRACT_META_EFFICIENCY)?,
        history_factory_raw: scalar(memory, at, market::CONTRACT_HISTORY_FACTORY)?,
        history_equipment_raw: scalar(memory, at, market::CONTRACT_HISTORY_EQUIPMENT)?,
        history_collected_raw: scalar(memory, at, market::CONTRACT_HISTORY_COLLECTED)?,
    })
}
fn read_country(
    memory: &impl Memory,
    base: usize,
    identity: engine_native_memory::country_identity::Country,
    manager: usize,
) -> Result<Country, &'static str> {
    authenticate_country(memory, base, identity)?;
    let mut native = market::Country::default();
    market::country(
        memory,
        base,
        identity,
        manager,
        market::Demand::ALL,
        &mut native,
    )?;
    RESET_WATCH
        .register(country_receiver(identity.id))
        .ok_or("market reset holder registration uncertified")?;
    for watch in &WATCHES {
        watch
            .register(country_receiver(identity.id))
            .ok_or("market holder registration uncertified")?;
    }
    Ok(Country {
        identity,
        manager,
        cutoff: OCCURRENCE.load(Ordering::Acquire),
        row: CountryMarketRow {
            country: identity.id,
            inventory: pool(
                native.stockpile.allow_zero,
                native
                    .stockpile
                    .equipment
                    .iter()
                    .map(|e| (e.variant, e.amount_raw)),
            )
            .into(),
            prices: levels(&native.price_levels).into(),
            subsidies: subsidies(&native.subsidies).into(),
            subsidy_total_raw: native.subsidy_total_raw,
            auto_send_market_access: native.auto_send_market_access,
            ..Default::default()
        },
        scratch: native,
    })
}
fn authenticate_country(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, ccountry::VTABLE_RVA)
        .ok_or("market country vtable address overflow")? as u64;
    if read::<u64>(memory, country.pointer) != Some(expected)
        || scalar::<u32>(memory, country.pointer, ccountry::ID)? != country.id
    {
        return Err("market country holder identity mismatch");
    }
    Ok(())
}
pub fn seed_country(memory: &impl Memory, base: usize, country: engine_native_memory::country_identity::Country, manager: usize) {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let mut state = STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let epoch = EPOCH.load(Ordering::Acquire);
    if state.epoch != epoch {
        *state = State::new(epoch);
    }
    state.bind_country_manager(manager);
    if state
        .countries
        .get(&country.id)
        .is_some_and(|held| held.identity == country && held.manager == manager)
        && !state.pending_countries.contains_key(&country.id)
    {
        return;
    }
    if state.countries.contains_key(&country.id)
        || state.pending_countries.contains_key(&country.id)
    {
        state
            .pending_countries
            .insert(country.id, (country, manager));
        return;
    }
    let cutoff = OCCURRENCE.load(Ordering::Acquire);
    match read_country(memory, base, country, manager) {
        Ok(held) => {
            state.countries.insert(country.id, held);
            state.unseeded_countries.remove(&country.id);
            state.changed[0].insert(country.id);
        }
        Err(reason) => {
            state.refusals[0] = Some(reason);
            state
                .pending_countries
                .insert(country.id, (country, manager));
        }
    }
    let lease_seed = (|| {
        let resources: u64 = scalar(memory, country.pointer, ccountry::RESOURCES)?;
        let exchanges = Array::read(memory, resources as usize + lease::OUTGOING, 8)
            .ok_or("market bootstrap exchange membership unreadable")?;
        for index in 0..exchanges.count {
            let pointer: u64 = read(
                memory,
                exchanges
                    .element(index, 8)
                    .ok_or("market exchange member address overflow")?,
            )
            .ok_or("market exchange member unreadable")?;
            let row = read_agreement(memory, base, pointer as usize)?;
            let key = (row.country, row.id);
            if row.last_fuel_delivered_raw != 0 {
                state.last_fuel.insert(key);
            }
            state.changed_agreements.insert(key);
            state.agreements.insert(key, Agreement { cutoff, row });
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = lease_seed {
        state.refusals[2] = Some(reason);
    }
    let request_seed = (|| {
        let pending = Array::read(
            memory,
            manager + PENDING_REQUESTS,
            engine_production::market_contract_manager::COUNTRY_VECTOR_STRIDE,
        )
        .ok_or("market pending request index unreadable")?;
        if country.canonical_slot < pending.count {
            let requests = Array::read(
                memory,
                pending
                    .element(
                        country.canonical_slot,
                        engine_production::market_contract_manager::COUNTRY_VECTOR_STRIDE,
                    )
                    .ok_or("market pending country address overflow")?,
                8,
            )
            .ok_or("market pending country list unreadable")?;
            for index in 0..requests.count {
                let pointer: u64 = read(
                    memory,
                    requests
                        .element(index, 8)
                        .ok_or("market pending member address overflow")?,
                )
                .ok_or("market pending member unreadable")?;
                let row = read_contract(memory, base, pointer as usize, true)?;
                state
                    .changed_contracts
                    .insert((row.country, row.id, row.kind));
                state.contracts.insert(
                    (row.country, row.id, row.kind),
                    Contract {
                        pointer: pointer as usize,
                        cutoff,
                        delivered: MarketPool::default(),
                        cost: 0,
                        row,
                    },
                );
            }
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = request_seed {
        state.refusals[1] = Some(reason);
    }
}
pub fn discover_country(
    memory: &impl Memory,
    base: usize,
    game: usize,
    country: engine_native_memory::country_identity::Country,
    _resources: usize,
) {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    {
        let mut state = STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let epoch = EPOCH.load(Ordering::Acquire);
        if state.epoch != epoch {
            *state = State::new(epoch);
        }
        if state
            .countries
            .get(&country.id)
            .is_some_and(|held| held.identity == country)
            && !state.pending_countries.contains_key(&country.id)
        {
            return;
        }
    }
    let manager = read::<u64>(memory, game + engine_native::cgame_state::MARKET_MANAGER);
    match manager {
        Some(manager) if manager != 0 => seed_country(memory, base, country, manager as usize),
        _ => refuse("market bootstrap manager unreadable"),
    }
}
pub fn seed_contract(memory: &impl Memory, base: usize, pointer: usize, id: u64, owner: u32) {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let mut state = STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let epoch = EPOCH.load(Ordering::Acquire);
    if state.epoch != epoch {
        *state = State::new(epoch);
    }
    if state
        .contracts
        .get(&(owner, id, 1))
        .is_some_and(|held| held.pointer != 0)
    {
        return;
    }
    if state.contracts_complete {
        state.discovered_contracts.insert((owner, id, 1));
        return;
    }
    let result = read_contract(memory, base, pointer, false).and_then(|row| {
        let at = pointer + market::CONTRACT_VARIABLES;
        super::register_variable_watch(at)
            .ok_or("market contract variable certificate unavailable")?;
        state
            .variable_owners
            .insert(at, (row.country, row.id, row.kind));
        let (delivered, cost) = worker::read_contract_total(memory, base, pointer, &row)?;
        Ok((row, delivered, cost))
    });
    match result {
        Ok((row, delivered, cost)) => {
            let key = (row.country, row.id, row.kind);
            if state
                .contracts
                .get(&key)
                .is_none_or(|held| held.pointer == 0)
            {
                state
                    .changed_contracts
                    .insert((row.country, row.id, row.kind));
                state.contracts.insert(
                    key,
                    Contract {
                        pointer,
                        cutoff: OCCURRENCE.load(Ordering::Acquire),
                        delivered,
                        cost,
                        row,
                    },
                );
            }
        }
        Err(reason) => state.refusals[1] = Some(reason),
    }
}
pub fn recover_contracts(memory: &impl Memory, base: usize, game: usize) {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    {
        let state = STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.complete
            || state.failure.is_some()
            || !state.publish_full
            || state.contracts_attempted
        {
            return;
        }
    }
    let result = (|| {
        let manager_address = game
            .checked_add(engine_native::cgame_state::MARKET_MANAGER)
            .ok_or("market recovery manager address overflow")?;
        let manager = read::<u64>(memory, manager_address)
            .filter(|&pointer| pointer != 0)
            .ok_or("market recovery manager unavailable")? as usize;
        let owned = Array::read(
            memory,
            manager
                .checked_add(engine_production::market_contract_manager::OWNED_CONTRACTS)
                .ok_or("market recovery membership address overflow")?,
            8,
        )
        .ok_or("market recovery contract membership unreadable")?;
        for index in 0..owned.count {
            let address = owned
                .element(index, 8)
                .ok_or("market recovery contract address overflow")?;
            let pointer = read::<u64>(memory, address)
                .filter(|&pointer| pointer != 0)
                .ok_or("market recovery contract member unreadable")?
                as usize;
            let id = scalar(
                memory,
                pointer,
                engine_production::cpurchase_contract::ID_TYPE,
            )?;
            let owner = scalar(
                memory,
                pointer,
                engine_production::cpurchase_contract::SELLER,
            )?;
            seed_contract(memory, base, pointer, id, owner);
        }
        Ok::<_, &'static str>(())
    })();
    finish_contract_seed(result.is_ok());
    if let Err(reason) = result {
        STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .refusals[1] = Some(reason);
    }
}

pub fn finish_seed(complete: bool) {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let mut state = STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let epoch = EPOCH.load(Ordering::Acquire);
    if state.epoch != epoch {
        *state = State::new(epoch);
    }
    if complete {
        state.complete = true;
    } else {
        state
            .failure
            .get_or_insert("market bootstrap census incomplete");
        RECOVERY.store(true, Ordering::SeqCst);
    }
}
pub fn refuse(reason: &'static str) {
    RECOVERY.store(true, Ordering::SeqCst);
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .failure = Some(reason);
}
fn refuse_country(state: &mut State, owner: u32, reason: &'static str) {
    state.refusals[0] = Some(reason);
    if let Some(held) = state.countries.get(&owner) {
        state
            .pending_countries
            .entry(owner)
            .or_insert((held.identity, held.manager));
    } else if !state.pending_countries.contains_key(&owner) {
        state.unseeded_countries.insert(owner);
    }
}
fn reset_country(state: &mut State, owner: u32) {
    if let Some(held) = state.countries.remove(&owner) {
        state
            .pending_countries
            .entry(owner)
            .or_insert((held.identity, held.manager));
    }
    state.dirty_inventory.remove(&owner);
    state.changed[0].insert(owner);
}
fn reseed_countries(state: &mut State, memory: &impl Memory, base: usize) -> BTreeSet<u32> {
    let pending = std::mem::take(&mut state.pending_countries);
    let mut acquired = BTreeSet::new();
    for (owner, (identity, manager)) in pending {
        match read_country(memory, base, identity, manager) {
            Ok(country) => {
                state.countries.insert(owner, country);
                state.unseeded_countries.remove(&owner);
                state.changed[0].insert(owner);
                state.dirty_inventory.remove(&owner);
                acquired.insert(owner);
            }
            Err(reason) => {
                state.refusals[0] = Some(reason);
                state.pending_countries.insert(owner, (identity, manager));
            }
        }
    }
    acquired
}
pub fn finish_contract_seed(complete: bool) {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let mut state = STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let epoch = EPOCH.load(Ordering::Acquire);
    if state.epoch != epoch {
        *state = State::new(epoch);
    }
    state.contracts_attempted = true;
    state.contracts_complete = complete;
    if !complete {
        state.refusals[1] = Some("market contract bootstrap census incomplete");
    }
}
fn apply(state: &mut State, event: &MarketEventRow) -> Result<(), &'static str> {
    if event.kind == AMENDMENT_COST
        || event.kind == CONTRACT_DELIVERED
        || event.kind == CONTRACT_PAYMENT
    {
        return Ok(());
    }
    if matches!(
        event.kind,
        REQUEST_CREATED | CONTRACT_CREATED | CONTRACT_REMOVED
    ) {
        let kind = if event.kind == CONTRACT_REMOVED {
            event.flags
        } else {
            u8::from(event.kind == CONTRACT_CREATED)
        };
        let key = (event.country, event.agreement, kind);
        if state
            .contracts
            .get(&key)
            .is_some_and(|held| event.occurrence < held.cutoff)
        {
            return Ok(());
        }
        state.discovered_contracts.remove(&key);
        state.contracts.remove(&key);
        state.variable_owners.retain(|_, owner| *owner != key);
        if event.kind == CONTRACT_REMOVED {
            state.dirty_contracts.remove(&key);
        }
        if event.kind == CONTRACT_REMOVED {
            state.admissions.remove(&key);
        }
        state.changed_contracts.insert(key);
        return Ok(());
    }
    let key = (event.country, event.agreement);
    if state
        .agreements
        .get(&key)
        .is_some_and(|held| event.occurrence < held.cutoff)
    {
        return Ok(());
    }
    if event.kind == REMOVED {
        state.agreements.remove(&key);
        state.dirty_agreements.remove(&key);
        state.last_fuel.remove(&key);
        state
            .changed_agreements
            .insert((event.country, event.agreement));
        return Ok(());
    }
    if event.kind == AMENDMENT {
        state.dirty_agreements.remove(&key);
        let held = state.agreements.entry(key).or_insert_with(|| Agreement {
            cutoff: 0,
            row: LendLeaseAgreementRow {
                country: event.country,
                id: event.agreement,
                receiver: event.receiver,
                ..Default::default()
            },
        });
        let row = &mut held.row;
        row.revision = event
            .occurrence
            .checked_add(1)
            .ok_or("market agreement revision identity exhausted")?;
        row.active_since = event.date;
        row.last_delivery = event.date;
        row.fuel_daily_raw = event.amount_raw;
        row.fuel_percentage_raw = event.auxiliary_raw;
        row.fuel_sent_raw = 0;
        row.monthly = MarketPool {
            allow_zero: event.flags & 1 != 0,
            ..Default::default()
        }
        .into();
        row.percentage = MarketPool {
            allow_zero: event.flags & 2 != 0,
            ..Default::default()
        }
        .into();
        row.once = MarketPool {
            allow_zero: event.flags & 4 != 0,
            ..Default::default()
        }
        .into();
        row.delivered = MarketPool {
            allow_zero: row.delivered.allow_zero,
            items: Vec::new(),
        }
        .into();
        row.sunk = MarketPool {
            allow_zero: row.sunk.allow_zero,
            items: Vec::new(),
        }
        .into();
    } else {
        let held = state
            .agreements
            .get_mut(&key)
            .ok_or("market event names an unobserved agreement revision")?;
        let row = &mut held.row;
        match event.kind {
            SCHEDULE => {
                let pool = match event.mode {
                    0 => &mut row.monthly,
                    1 => &mut row.percentage,
                    2 => &mut row.once,
                    _ => return Err("market schedule mode unsupported"),
                };
                if pool.items.len() != event.ordinal as usize {
                    return Err("market schedule item ordinal discontinuity");
                }
                pool.items.push(MarketEquipment {
                    variant: event.variant,
                    amount_raw: event.amount_raw,
                });
            }
            FUEL => match event.mode {
                0 => {
                    row.fuel_sent_raw = row.fuel_sent_raw.wrapping_add(event.amount_raw);
                    row.last_fuel_delivered_raw = event.amount_raw;
                    if event.amount_raw != 0 {
                        state.last_fuel.insert(key);
                    }
                }
                1 => row.fuel_sunk_raw = row.fuel_sunk_raw.wrapping_add(event.auxiliary_raw),
                _ => return Err("market fuel store mode unsupported"),
            },
            _ => return Err("market event kind unsupported"),
        }
    }
    state
        .changed_agreements
        .insert((event.country, event.agreement));
    Ok(())
}
fn pool_domain(mode: u8) -> usize {
    match mode {
        INVENTORY_MODE => 0,
        ROUTE_MODE => 1,
        _ => 2,
    }
}
fn event_domain(event: &MarketEventRow) -> usize {
    match event.kind {
        INVENTORY => 0,
        POOL_VALUE | POOL_END => pool_domain(event.mode),
        REQUEST_CREATED | CONTRACT_CREATED | CONTRACT_REMOVED | CONTRACT_DELIVERED
        | CONTRACT_PAYMENT => 1,
        ROUTE_BEGIN..=ROUTE_OPERAND => 1,
        _ => 2,
    }
}
fn apply_observed(state: &mut State, event: &MarketEventRow) {
    let domain = event_domain(event);
    if (state.refusals[domain].is_none()
        || matches!(
            event.kind,
            REQUEST_CREATED | CONTRACT_CREATED | CONTRACT_REMOVED | REMOVED | AMENDMENT
        ))
        && let Err(reason) = apply(state, event)
    {
        state.refusals[domain] = Some(reason);
    }
}
fn reset_last_fuel(state: &mut State, hour: i32, full: bool) {
    if state.fuel_hour == Some(hour) {
        return;
    }
    state.fuel_hour = Some(hour);
    if full {
        return;
    }
    for key in std::mem::take(&mut state.last_fuel) {
        if let Some(held) = state.agreements.get_mut(&key)
            && held.row.last_fuel_delivered_raw != 0
        {
            held.row.last_fuel_delivered_raw = 0;
            state.changed_agreements.insert(key);
        }
    }
}
fn acquire_admissions(state: &mut State, memory: &impl Memory, base: usize) {
    for (key, pointer) in std::mem::take(&mut state.admissions) {
        if state.refusals[1].is_some() || state.contracts.contains_key(&key) {
            continue;
        }
        let acquired = read_contract(memory, base, pointer, key.2 == 0).and_then(|row| {
            if (row.country, row.id, row.kind) != key {
                return Err("market admitted contract identity changed");
            }
            if key.2 == 1 {
                let at = pointer + market::CONTRACT_VARIABLES;
                super::register_variable_watch(at)
                    .ok_or("market new contract variable certificate unavailable")?;
                state.variable_owners.insert(at, key);
            }
            let cost = if key.2 == 1 {
                worker::read_cost(memory, base, pointer, &row)?
            } else {
                0
            };
            Ok((row, cost))
        });
        match acquired {
            Ok((row, cost)) => {
                state.contracts.insert(
                    key,
                    Contract {
                        pointer,
                        cutoff: OCCURRENCE.load(Ordering::Acquire),
                        delivered: MarketPool::default(),
                        cost,
                        row,
                    },
                );
                state.changed_contracts.insert(key);
            }
            Err(reason) => state.refusals[1] = Some(reason),
        }
    }
}
pub fn unavailable_amounts(events: &[MarketEventRow], output: &mut Vec<MarketEventRow>) {
    let mut state = State::new(0);
    for event in events {
        if event.kind == AMENDMENT_COST && event.mode == HOLDER_MARK {
            match event.flags {
                AGREEMENT_MARK => worker::delivery_unavailable(
                    &mut state,
                    *event,
                    false,
                    event.ordinal as u8,
                    2,
                    output,
                ),
                CONTRACT_MARK => worker::delivery_unavailable(
                    &mut state,
                    *event,
                    true,
                    event.ordinal as u8,
                    2,
                    output,
                ),
                _ => {}
            }
        } else if event.kind == FINAL_END {
            worker::delivery_unavailable(
                &mut state,
                *event,
                false,
                READ_DELIVERED | READ_SUNK,
                2,
                output,
            );
        } else if event.kind == AMENDMENT_COST
            && event.mode == FINAL_CONTRACT
            && (event.amount_raw != 0 || event.ordinal != 0)
        {
            worker::delivery_unavailable(&mut state, *event, true, READ_PURCHASE_TOTAL, 2, output);
        }
    }
}
pub struct Changes {
    pub owners: [Vec<u32>; 2],
    pub agreement_keys: Vec<u128>,
    pub contract_keys: Vec<u128>,
    pub refusals: [Option<&'static str>; 3],
    pub events: Vec<MarketEventRow>,
    pub countries: Vec<CountryMarketRow>,
    pub contracts: Vec<MarketContractTermsRow>,
    pub agreements: Vec<LendLeaseAgreementRow>,
    pub full: bool,
    pub countries_full: bool,
}
fn observe_interval(
    state: &mut State,
    events: &[MarketEventRow],
    hour: i32,
    full: bool,
    observed: &mut Vec<MarketEventRow>,
) {
    for event in events {
        if !full {
            if event.kind == AMENDMENT_COST
                && matches!(
                    event.mode,
                    HOLDER_MARK | FINAL_CONTRACT | COUNTRY_RESET | ADMISSION
                )
                || (FINAL_BEGIN..=FINAL_END).contains(&event.kind)
            {
                if let Err(reason) = worker::observe(state, event, observed) {
                    let domain = if event.kind == AMENDMENT_COST && event.flags == CONTRACT_MARK
                        || event.mode == FINAL_CONTRACT
                    {
                        1
                    } else {
                        2
                    };
                    state.refusals[domain] = Some(reason);
                    worker::delivery_unavailable(
                        state,
                        *event,
                        domain == 1,
                        if domain == 1 {
                            READ_PURCHASE_TOTAL
                        } else {
                            READ_DELIVERED | READ_SUNK
                        },
                        3,
                        observed,
                    );
                }
            } else {
                worker::erased_marks(state, event, observed);
                apply_observed(state, event);
            }
        }
        if event.kind != AMENDMENT_COST && event.kind < FINAL_BEGIN {
            observed.push(*event);
        }
    }
    if !state.terminal.is_empty() {
        state.refusals[2] = Some("market final cumulative pools incomplete");
        for ((country, agreement, occurrence), _) in std::mem::take(&mut state.terminal) {
            worker::delivery_unavailable(
                state,
                MarketEventRow {
                    country,
                    agreement,
                    occurrence,
                    hour,
                    ..Default::default()
                },
                false,
                READ_DELIVERED | READ_SUNK,
                3,
                observed,
            );
        }
    }
}
fn reread_variables(
    state: &mut State,
    memory: &impl Memory,
    base: usize,
    at: usize,
) -> Result<(), &'static str> {
    let Some(key) = state.variable_owners.get(&at).copied() else {
        return Ok(());
    };
    let Some(held) = state.contracts.get_mut(&key) else {
        return Ok(());
    };
    if held.pointer + market::CONTRACT_VARIABLES != at
        || read::<u64>(
            memory,
            held.pointer + engine_production::cpurchase_contract::ID_TYPE,
        ) != Some(held.row.id)
    {
        return Err("market variable holder identity changed");
    }
    let rows = read_variables(memory, base, at)?;
    held.row.variables = rows.into();
    let seed = scalar(memory, at, engine_scripts::cvariables::SEED);
    state.changed_contracts.insert(key);
    held.row.variable_state = seed?;
    Ok(())
}
pub fn acquire(
    memory: &impl Memory,
    base: usize,
    events: &[MarketEventRow],
    hour: i32,
    full: bool,
) -> Result<Changes, &'static str> {
    let mut state = STATE.lock().map_err(|_| "market held state poisoned")?;
    if state.epoch != EPOCH.load(Ordering::Acquire) || !state.complete {
        return Err("market initial census not complete");
    }
    if let Some(reason) = state.failure {
        return Err(reason);
    }
    if !state.contracts_complete {
        state.refusals[1] = Some("market contract census incomplete");
    }
    if full {
        state.admissions.clear();
    }
    reset_last_fuel(&mut state, hour, full);
    let mut observed = Vec::new();
    observe_interval(&mut state, events, hour, full, &mut observed);
    for key in std::mem::take(&mut state.discovered_contracts) {
        if !state.admissions.contains_key(&key) && !state.contracts.contains_key(&key) {
            state.refusals[1] = Some("market new contract has no observed birth");
        }
    }
    let reset_holdings = reseed_countries(&mut state, memory, base);
    acquire_admissions(&mut state, memory, base);
    worker::acquire_marked(&mut state, memory, base, &mut observed);
    let mut variable_failure = None;
    let every_holder = take_unobserved_variables();
    if VARIABLES_WATCH
        .take(|at| {
            if let Err(reason) = reread_variables(&mut state, memory, base, at) {
                variable_failure = Some(reason);
            }
        })
        .is_none()
    {
        variable_failure = Some("market contract variable coverage unavailable");
    }
    if every_holder {
        let holders: Vec<usize> = state.variable_owners.keys().copied().collect();
        for at in holders {
            if let Err(reason) = reread_variables(&mut state, memory, base, at) {
                variable_failure = Some(reason);
            }
        }
    }
    if let Some(reason) = variable_failure {
        state.refusals[1] = Some(reason);
    }
    for (domain, watch) in WATCHES.iter().enumerate() {
        let mut failure = None;
        if watch
            .take(|owner| {
                let owner = ((owner >> 3) - 1) as u32;
                if reset_holdings.contains(&owner) || state.pending_countries.contains_key(&owner) {
                    return;
                }
                let Some(held) = state.countries.get_mut(&owner) else {
                    refuse_country(&mut state, owner, "market marked country has no bootstrap");
                    return;
                };
                if let Err(reason) = authenticate_country(memory, base, held.identity) {
                    refuse_country(&mut state, owner, reason);
                    return;
                }
                let demand = market::Demand {
                    price_levels: domain == PRICES,
                    subsidies: domain == SUBSIDIES,
                    automation: domain == AUTOMATION,
                    ..Default::default()
                };
                match market::country(
                    memory,
                    base,
                    held.identity,
                    held.manager,
                    demand,
                    &mut held.scratch,
                ) {
                    Ok(()) => {
                        match domain {
                            PRICES => held.row.prices = levels(&held.scratch.price_levels).into(),
                            SUBSIDIES => {
                                held.row.subsidies = subsidies(&held.scratch.subsidies).into();
                                held.row.subsidy_total_raw = held.scratch.subsidy_total_raw;
                            }
                            AUTOMATION => {
                                held.row.auto_send_market_access =
                                    held.scratch.auto_send_market_access;
                            }
                            _ => unreachable!(),
                        }
                        state.changed[0].insert(owner);
                    }
                    Err(reason) => refuse_country(&mut state, owner, reason),
                }
            })
            .is_none()
        {
            if !state.uncertified_watches[domain] {
                state.recertify_countries();
            }
            state.uncertified_watches[domain] = true;
            failure = Some("market writer certificate unavailable");
        } else {
            state.uncertified_watches[domain] = false;
        }
        if let Some(reason) = failure {
            state.refusals[0] = Some(reason);
        }
    }
    if !state.unseeded_countries.is_empty() {
        state.refusals[0].get_or_insert("market marked country has no bootstrap");
    }
    if full {
        let owners: Vec<_> = state.countries.keys().copied().collect();
        for domain in &mut state.changed {
            domain.extend(owners.iter().copied());
        }
        let contract_keys: Vec<_> = state.contracts.keys().copied().collect();
        state.changed_contracts.extend(contract_keys);
        let keys: Vec<_> = state.agreements.keys().copied().collect();
        state.changed_agreements.extend(keys);
    }
    let mut changes = publish_changes(&mut state, full);
    changes.events = observed;
    Ok(changes)
}
fn publish_changes(state: &mut State, full: bool) -> Changes {
    state.publish_full = false;
    let countries_full = full || state.publish_countries_full;
    if countries_full && state.refusals[0].is_none() {
        state.changed[0].extend(state.countries.keys().copied());
    }
    if state.refusals[0].is_none() {
        state.publish_countries_full = false;
    }
    let owners = state
        .changed
        .each_ref()
        .map(|owners| owners.iter().copied().collect::<Vec<_>>());
    let countries = owners[0]
        .iter()
        .filter_map(|owner| state.countries.get(owner).map(|held| held.row.clone()))
        .collect();
    let contracts = state
        .changed_contracts
        .iter()
        .filter_map(|key| state.contracts.get(key).map(|held| held.row.clone()))
        .collect();
    let contract_keys = state
        .changed_contracts
        .iter()
        .map(|&(country, id, kind)| {
            (u128::from(country) << 72) | (u128::from(id) << 8) | u128::from(kind)
        })
        .collect();
    state.changed_contracts.clear();
    let agreements = state
        .changed_agreements
        .iter()
        .filter_map(|key| state.agreements.get(key).map(|held| held.row.clone()))
        .collect();
    let agreement_keys = state
        .changed_agreements
        .iter()
        .map(|&(country, id)| (u128::from(country) << 64) | u128::from(id))
        .collect();
    state.changed_agreements.clear();
    for owners in &mut state.changed {
        owners.clear();
    }
    Changes {
        owners,
        agreement_keys,
        contract_keys,
        refusals: state.refusals,
        events: Vec::new(),
        countries,
        contracts,
        agreements,
        full,
        countries_full,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn test_census(rows: [replay_data::rows::TableRows; 3]) {
    use replay_data::rows::TableRows;
    let mut state = STATE.lock().unwrap();
    let epoch = EPOCH.load(Ordering::Acquire);
    if state.epoch != epoch {
        *state = State::new(epoch);
    }
    for rows in rows {
        match rows {
            TableRows::CountryMarket(rows) => {
                for row in rows {
                    state.changed[0].insert(row.country);
                    state.countries.insert(
                        row.country,
                        Country {
                            identity: engine_native_memory::country_identity::Country {
                                pointer: usize::MAX,
                                id: row.country,
                                canonical_slot: row.country,
                            },
                            manager: 0,
                            cutoff: 0,
                            row,
                            scratch: Default::default(),
                        },
                    );
                }
            }
            TableRows::MarketContractTerms(rows) => {
                for row in rows {
                    let key = (row.country, row.id, row.kind);
                    state.changed_contracts.insert(key);
                    state.contracts.insert(
                        key,
                        Contract {
                            pointer: 0,
                            cutoff: 0,
                            row,
                            delivered: Default::default(),
                            cost: 1,
                        },
                    );
                }
            }
            TableRows::LendLeaseAgreements(rows) => {
                for row in rows {
                    let key = (row.country, row.id);
                    state.changed_agreements.insert(key);
                    state.agreements.insert(key, Agreement { cutoff: 0, row });
                }
            }
            _ => panic!("unexpected synthetic census table"),
        }
    }
    drop(state);
    finish_seed(true);
    finish_contract_seed(true);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreign_giver_agreement_is_recorded_under_its_resources_owner() {
        use engine_native::{
            ccountry::synthetic::World,
            cequipment_variant::pool as native_pool,
        };
        let mut world = World::new(0, 2, 0x20);
        let resources = world.alloc(0x30);
        world.u64(
            resources,
            engine_resources::ccountry_resources::VTABLE_RVA.0 as u64,
        );
        world.u32(
            resources + engine_resources::ccountry_resources::OWNER,
            5001,
        );
        let pointer = world.alloc(lease::SIZE);
        let id = (17_u64 << 32) | 9001;
        world.u64(pointer, lease::VTABLE_RVA.0 as u64);
        world.u64(pointer + lease::HANDLE, id);
        world.u64(pointer + lease::GIVER_RESOURCES, resources as u64);
        world.u32(pointer + lease::RECEIVER, 4999);
        for pool in [
            lease::MONTHLY_NEED,
            lease::PERCENTAGE_NEED,
            lease::ONCE_NEED,
            lease::DELIVERED,
            lease::SUNK,
        ] {
            world.u64(pointer + pool, native_pool::VTABLE_RVA.0 as u64);
        }
        let row = read_agreement(&world.memory(), 0, pointer).unwrap();
        assert_eq!((row.country, row.id, row.receiver), (5001, id, 4999));
    }
    #[test]
    fn terminal_differences_precede_country_reset_and_post_reset_birth_admission() {
        let mut state = agreement();
        state.contracts.insert(
            (5000, 18, 1),
            Contract {
                pointer: usize::MAX,
                cutoff: 0,
                cost: 100_000,
                delivered: pool(false, [(9, 9_900_000)]),
                row: MarketContractTermsRow {
                    country: 5000,
                    id: 18,
                    kind: 1,
                    equipment: pool(false, [(9, 20_000_000)]).into(),
                    ..Default::default()
                },
            },
        );
        let rows = [
            event(FINAL_BEGIN, 1),
            MarketEventRow {
                variant: 9,
                amount_raw: 100,
                ..event(FINAL_ITEM, 1)
            },
            event(FINAL_END, 1),
            event(REMOVED, 2),
            MarketEventRow {
                mode: FINAL_CONTRACT,
                flags: 1,
                agreement: 18,
                amount_raw: 50_000,
                auxiliary_raw: 100_000,
                ..event(AMENDMENT_COST, 3)
            },
            MarketEventRow {
                agreement: 18,
                flags: 1,
                ..event(CONTRACT_REMOVED, 4)
            },
            MarketEventRow {
                mode: COUNTRY_RESET,
                ..event(AMENDMENT_COST, 5)
            },
            MarketEventRow {
                agreement: 19,
                ..event(CONTRACT_CREATED, 6)
            },
            MarketEventRow {
                mode: ADMISSION,
                flags: 1,
                agreement: 19,
                total_raw: 1234,
                ..event(AMENDMENT_COST, 6)
            },
        ];
        let mut output = Vec::new();
        observe_interval(&mut state, &rows, 7, false, &mut output);
        assert_eq!(
            output
                .iter()
                .filter(|row| matches!(row.kind, DELIVERED | CONTRACT_DELIVERED))
                .map(|row| (row.kind, row.amount_raw))
                .collect::<Vec<_>>(),
            [(DELIVERED, 1), (CONTRACT_DELIVERED, 100_000)]
        );
        assert!(state.agreements.is_empty());
        assert!(state.contracts.is_empty());
        assert_eq!(state.admissions.get(&(5000, 19, 1)), Some(&1234));
        assert_eq!(state.refusals, [None; 3]);
    }

    #[test]
    fn country_reset_keeps_surviving_holder_baselines_and_their_pending_marks() {
        let mut state = agreement();
        let allocation = state.agreements[&(5000, 17)].row.delivered.items.as_ptr();
        state.contracts.insert(
            (5000, 18, 1),
            Contract {
                pointer: usize::MAX,
                cutoff: 0,
                row: MarketContractTermsRow {
                    country: 5000,
                    id: 18,
                    kind: 1,
                    equipment: pool(false, [(9, 200)]).into(),
                    ..Default::default()
                },
                delivered: pool(false, [(9, 99)]),
                cost: 200,
            },
        );
        let manifest = state.contracts[&(5000, 18, 1)].row.equipment.items.as_ptr();
        state.dirty_agreements.insert(
            (5000, 17),
            (event(DELIVERED, 1), usize::MAX, READ_DELIVERED),
        );
        state.dirty_contracts.insert(
            (5000, 18, 1),
            (
                event(CONTRACT_DELIVERED, 1),
                usize::MAX,
                READ_PURCHASE_TOTAL,
            ),
        );
        state.admissions.insert((5000, 19, 1), 1234);
        let mut output = Vec::new();
        observe_interval(
            &mut state,
            &[MarketEventRow {
                mode: COUNTRY_RESET,
                ..event(AMENDMENT_COST, 2)
            }],
            7,
            false,
            &mut output,
        );
        assert!(output.is_empty());
        assert_eq!(
            state.agreements[&(5000, 17)].row.delivered.items[0].amount_raw,
            99
        );
        assert_eq!(
            state.agreements[&(5000, 17)].row.delivered.items.as_ptr(),
            allocation
        );
        assert_eq!(
            state.contracts[&(5000, 18, 1)].delivered.items[0].amount_raw,
            99
        );
        assert_eq!(
            state.contracts[&(5000, 18, 1)].row.equipment.items.as_ptr(),
            manifest
        );
        assert!(state.dirty_agreements.contains_key(&(5000, 17)));
        assert!(state.dirty_contracts.contains_key(&(5000, 18, 1)));
        assert_eq!(state.admissions.get(&(5000, 19, 1)), Some(&1234));
    }

    #[test]
    fn removals_clear_private_marks_even_after_their_state_table_refuses() {
        let mut state = agreement();
        state.refusals[1] = Some("synthetic contract state refusal");
        state.refusals[2] = Some("synthetic agreement state refusal");
        state.admissions.insert((5000, 18, 1), usize::MAX);
        state.dirty_contracts.insert(
            (5000, 18, 1),
            (
                event(CONTRACT_DELIVERED, 1),
                usize::MAX,
                READ_PURCHASE_TOTAL,
            ),
        );
        state.dirty_agreements.insert(
            (5000, 17),
            (event(DELIVERED, 1), usize::MAX, READ_DELIVERED),
        );
        apply_observed(&mut state, &event(REMOVED, 2));
        apply_observed(
            &mut state,
            &MarketEventRow {
                agreement: 18,
                flags: 1,
                ..event(CONTRACT_REMOVED, 2)
            },
        );
        let mut output = Vec::new();
        worker::acquire_marked(&mut state, &NoNativeRead, 0, &mut output);
        assert!(state.dirty_agreements.is_empty());
        assert!(state.dirty_contracts.is_empty());
        assert!(state.admissions.is_empty());
        assert!(output.is_empty());
    }

    #[test]
    fn removal_without_the_expected_terminal_witness_names_its_erased_delivery_mark() {
        let mut state = agreement();
        let rows = [
            MarketEventRow {
                mode: HOLDER_MARK,
                flags: AGREEMENT_MARK,
                ordinal: u32::from(READ_DELIVERED),
                total_raw: -1,
                ..event(AMENDMENT_COST, 1)
            },
            event(REMOVED, 2),
        ];
        let mut output = Vec::new();
        observe_interval(&mut state, &rows, 7, false, &mut output);
        assert_eq!(output.len(), 2);
        assert_eq!(output[1].kind, REMOVED);
        assert_eq!(output[0].kind, DELIVERY_UNAVAILABLE);
        assert_eq!(output[0].agreement, 17);
        assert_eq!(output[0].ordinal, 5);
        assert!(
            output[0]
                .delivery_unavailable_reason()
                .unwrap()
                .contains("final totals witness")
        );
        worker::acquire_marked(&mut state, &NoNativeRead, 0, &mut Vec::new());
    }

    fn agreement() -> State {
        let mut state = State::new(0);
        state.agreements.insert(
            (5000, 17),
            Agreement {
                cutoff: 0,
                row: LendLeaseAgreementRow {
                    country: 5000,
                    id: 17,
                    receiver: 4999,
                    fuel_sunk_raw: 123,
                    last_fuel_delivered_raw: 23,
                    delivered: MarketPool {
                        items: vec![MarketEquipment {
                            variant: 9,
                            amount_raw: 99,
                        }],
                        ..Default::default()
                    }
                    .into(),
                    ..Default::default()
                },
            },
        );
        state
    }
    fn event(kind: u8, occurrence: u64) -> MarketEventRow {
        MarketEventRow {
            country: 5000,
            agreement: 17,
            receiver: 4999,
            kind,
            occurrence,
            ..Default::default()
        }
    }
    struct NoNativeRead;
    impl Memory for NoNativeRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("certified occurrence must not read native memory");
        }
    }
    #[test]
    fn surviving_request_reads_its_owned_definition_once_on_the_worker() {
        const HOLDER: usize = 0x100;
        const HEAD: usize = 0x600;
        let mut bytes = vec![0; 0x1000];
        fn put(bytes: &mut [u8], at: usize, value: u64) {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        put(&mut bytes, HOLDER, REQUEST_VTABLE_RVA.0 as u64);
        put(
            &mut bytes,
            HOLDER + engine_production::cpurchase_contract::ID_TYPE,
            17,
        );
        let seller = HOLDER + engine_production::cpurchase_contract::SELLER;
        bytes[seller..seller + 4].copy_from_slice(&5000_u32.to_le_bytes());
        for at in [
            HOLDER + market::DRAFT,
            HOLDER + market::DRAFT + market::DRAFT_EQUIPMENT,
        ] {
            put(
                &mut bytes,
                at,
                engine_native::cequipment_variant::pool::VTABLE_RVA.0 as u64,
            );
        }
        put(
            &mut bytes,
            HOLDER + market::DRAFT + market::DRAFT_PRICE_LEVELS,
            HEAD as u64,
        );
        for offset in [0, 8, 16] {
            put(&mut bytes, HEAD + offset, HEAD as u64);
        }
        bytes[HEAD + 25] = 1;
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut state = State::new(0);
        state.admissions.insert((5000, 17, 0), HOLDER);
        apply_observed(&mut state, &event(REQUEST_CREATED, 1));
        acquire_admissions(&mut state, &memory, 0);
        assert_eq!(state.refusals[1], None);
        assert_eq!(state.contracts[&(5000, 17, 0)].pointer, HOLDER);
        assert!(
            state.contracts[&(5000, 17, 0)]
                .row
                .equipment
                .items
                .is_empty()
        );
        state.admissions.insert((5000, 17, 0), usize::MAX);
        acquire_admissions(&mut state, &NoNativeRead, 0);
        assert!(state.admissions.is_empty());
    }
    #[test]
    fn fuel_operands_wrap_and_last_delivery_resets_only_once_in_the_next_hour() {
        let mut state = agreement();
        state.fuel_hour = Some(7);
        state.last_fuel.insert((5000, 17));
        state
            .agreements
            .get_mut(&(5000, 17))
            .unwrap()
            .row
            .fuel_sent_raw = i64::MAX;
        apply(
            &mut state,
            &MarketEventRow {
                amount_raw: 1,
                ..event(FUEL, 1)
            },
        )
        .unwrap();
        apply(
            &mut state,
            &MarketEventRow {
                mode: 1,
                auxiliary_raw: -7,
                ..event(FUEL, 2)
            },
        )
        .unwrap();
        let row = &state.agreements[&(5000, 17)].row;
        assert_eq!(row.fuel_sent_raw, i64::MIN);
        assert_eq!(row.fuel_sunk_raw, 116);
        assert_eq!(row.last_fuel_delivered_raw, 1);
        reset_last_fuel(&mut state, 7, false);
        assert_eq!(state.agreements[&(5000, 17)].row.last_fuel_delivered_raw, 1);
        state.changed_agreements.clear();
        reset_last_fuel(&mut state, 8, false);
        assert_eq!(state.agreements[&(5000, 17)].row.last_fuel_delivered_raw, 0);
        assert_eq!(state.changed_agreements.len(), 1);
        state.changed_agreements.clear();
        reset_last_fuel(&mut state, 8, false);
        reset_last_fuel(&mut state, 9, false);
        assert!(state.changed_agreements.is_empty());
        state
            .agreements
            .get_mut(&(5000, 17))
            .unwrap()
            .row
            .last_fuel_delivered_raw = 23;
        state.last_fuel.insert((5000, 17));
        reset_last_fuel(&mut state, 10, true);
        assert_eq!(
            state.agreements[&(5000, 17)].row.last_fuel_delivered_raw,
            23
        );
    }
    #[test]
    fn one_fuel_write_publishes_one_of_many_agreements_of_the_same_giver_with_shared_pools() {
        const AGREEMENTS: usize = 32;
        let mut state = State::new(0);
        let pool: replay_data::rows::SharedMarketPool = MarketPool {
            items: vec![
                MarketEquipment {
                    variant: 31,
                    amount_raw: 99
                };
                AGREEMENTS
            ],
            ..Default::default()
        }
        .into();
        let allocation = pool.items.as_ptr();
        for id in 0..AGREEMENTS as u64 {
            state.agreements.insert(
                (5000, id),
                Agreement {
                    cutoff: 0,
                    row: LendLeaseAgreementRow {
                        country: 5000,
                        id,
                        monthly: pool.clone(),
                        percentage: pool.clone(),
                        once: pool.clone(),
                        delivered: pool.clone(),
                        sunk: pool.clone(),
                        ..Default::default()
                    },
                },
            );
        }
        apply(
            &mut state,
            &MarketEventRow {
                amount_raw: 77,
                ..event(FUEL, 1)
            },
        )
        .unwrap();
        let changes = publish_changes(&mut state, false);
        assert_eq!(changes.agreement_keys, [(5000_u128 << 64) | 17]);
        assert_eq!(changes.agreements.len(), 1);
        let row = &changes.agreements[0];
        for pool in [
            &row.monthly,
            &row.percentage,
            &row.once,
            &row.delivered,
            &row.sunk,
        ] {
            assert_eq!(pool.items.as_ptr(), allocation);
        }
        assert_eq!(row.fuel_sent_raw, 77);
        assert_eq!(state.agreements[&(5000, 18)].row.fuel_sent_raw, 0);
        assert!(publish_changes(&mut state, false).agreements.is_empty());
        for agreement in 0..AGREEMENTS as u64 {
            apply_observed(
                &mut state,
                &MarketEventRow {
                    agreement,
                    ..event(REMOVED, 2)
                },
            );
        }
        let changes = publish_changes(&mut state, false);
        assert_eq!(changes.agreement_keys.len(), AGREEMENTS);
        assert!(changes.agreements.is_empty());
    }
    #[test]
    fn market_first_amendment_has_a_revision_distinct_from_the_census() {
        let mut state = agreement();
        assert_eq!(state.agreements[&(5000, 17)].row.revision, 0);
        apply(&mut state, &event(AMENDMENT, 0)).unwrap();
        assert_eq!(state.agreements[&(5000, 17)].row.revision, 1);
    }

    #[test]
    fn final_totals_preserve_the_unacquired_increment_before_each_reset_and_removal() {
        let mut state = agreement();
        let mut output = Vec::new();
        for (occurrence, total) in [(1, 150), (3, 20)] {
            for row in [
                MarketEventRow {
                    flags: 3,
                    ..event(FINAL_BEGIN, occurrence)
                },
                MarketEventRow {
                    variant: 9,
                    amount_raw: total,
                    ..event(FINAL_ITEM, occurrence)
                },
                event(FINAL_END, occurrence),
            ] {
                worker::observe(&mut state, &row, &mut output).unwrap();
            }
            if occurrence == 1 {
                apply_observed(&mut state, &event(AMENDMENT, 2));
                assert!(state.agreements[&(5000, 17)].row.delivered.allow_zero);
                assert!(state.agreements[&(5000, 17)].row.sunk.allow_zero);
                assert!(state.agreements[&(5000, 17)].row.delivered.items.is_empty());
            } else {
                apply_observed(&mut state, &event(REMOVED, 4));
            }
        }
        assert_eq!(
            output
                .iter()
                .map(|row| (row.kind, row.amount_raw, row.total_raw))
                .collect::<Vec<_>>(),
            [(DELIVERED, 51, 150), (DELIVERED, 20, 20)]
        );
        assert!(output.iter().all(|row| row.flags == DERIVED_TOTAL));
        assert!(state.agreements.is_empty());
        assert!(state.terminal.is_empty());
        assert_eq!(state.refusals, [None; 3]);
    }
    #[test]
    fn constructor_order_marks_then_transient_removal_never_dereference_the_holder() {
        let mut state = State::new(0);
        state.admissions.insert((5000, 17, 1), usize::MAX);
        worker::observe(
            &mut state,
            &MarketEventRow {
                kind: AMENDMENT_COST,
                mode: HOLDER_MARK,
                flags: CONTRACT_MARK,
                total_raw: -1,
                ..event(CONTRACT_STATE, 1)
            },
            &mut Vec::new(),
        )
        .unwrap();
        apply_observed(&mut state, &event(CONTRACT_CREATED, 2));
        apply_observed(
            &mut state,
            &MarketEventRow {
                flags: 1,
                ..event(CONTRACT_REMOVED, 3)
            },
        );
        acquire_admissions(&mut state, &NoNativeRead, 0);
        worker::acquire_marked(&mut state, &NoNativeRead, 0, &mut Vec::new());
        assert!(state.contracts.is_empty());
        assert!(state.dirty_contracts.is_empty());
        assert_eq!(state.refusals, [None; 3]);
    }
    #[test]
    fn changed_holder_read_failure_is_isolated_from_agreements_and_event_observation() {
        struct Missing;
        impl Memory for Missing {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        let mut state = agreement();
        state.contracts.insert(
            (5000, 17, 1),
            Contract {
                pointer: 0,
                cutoff: 0,
                row: MarketContractTermsRow::default(),
                delivered: MarketPool::default(),
                cost: 0,
            },
        );
        worker::observe(
            &mut state,
            &MarketEventRow {
                kind: AMENDMENT_COST,
                mode: HOLDER_MARK,
                flags: CONTRACT_MARK,
                ..event(CONTRACT_STATE, 1)
            },
            &mut Vec::new(),
        )
        .unwrap();
        worker::acquire_marked(&mut state, &Missing, 0, &mut Vec::new());
        apply_observed(
            &mut state,
            &MarketEventRow {
                amount_raw: 19,
                ..event(FUEL, 2)
            },
        );
        assert_eq!(
            state.refusals,
            [None, Some("market marked contract identity changed"), None]
        );
        assert_eq!(state.agreements[&(5000, 17)].row.fuel_sent_raw, 19);
        assert!(state.failure.is_none());
    }
}
