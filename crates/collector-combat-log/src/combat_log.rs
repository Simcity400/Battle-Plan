use crate::{
    clock::qpc, slicing::SliceSelection, table_capture::CaptureRows, table_conversion::table,
};
use engine_allocation as allocation;
use engine_land_inputs::{
    ncombat_log_cequipment_loss::month_bucket, ncombat_log_cmanager as native,
};
use engine_native_memory::{Memory, walk::Outcome};
use engine_resources::clend_lease_exchange as lend_lease;
use replay_data::{
    cadence::SKIPPED_REASON, cadence::STAGGER_PERIOD, cadence::should_walk,
    rows::CombatLogEquipmentRow, rows::CombatLogStateRow, rows::LendLeaseEquipmentRow,
    rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};

pub const N: usize = 3 + crate::history_capture::N;
pub const TABLES: [TableId; N] = [
    TableId::CombatLogEquipment,
    TableId::CombatLogState,
    TableId::LendLeaseEquipment,
    TableId::CombatLogGroups,
    TableId::CombatLogMonths,
    TableId::CombatLogManpower,
    TableId::CombatLogTemplates,
    TableId::CombatLogReferences,
    TableId::CombatLogBattles,
    TableId::CombatLogEnemyEquipment,
    TableId::CombatLogBattleSides,
    TableId::CombatLogBattleEquipment,
    TableId::CombatLogBattleModifiers,
    TableId::CombatLogBattleCountries,
];

pub(crate) fn release_retained_history() {
    with_store(|store| {
        store.retained = crate::history_capture::State::new();
        store.countries = Vec::new();
    });
    crate::history_capture::release_conversion();
}

pub const PRUNE_HOUR: i32 = 23;

pub const RETAIN_HOURS: i32 = 24;
const NEVER: i32 = i32::MIN;

#[derive(Clone, Copy)]
struct Final {
    row: native::Row,
    published: Option<i32>,
}

struct Store {
    retained: crate::history_capture::State,
    signal: Option<i64>,
    hour: Option<i32>,
    finals: Vec<Final>,
    countries: Vec<CountryRead>,
}

struct CountryRead {
    country: u32,
    last_whole: i32,
    visited: i32,
    sealed: native::SealedMonths,
}

#[derive(Clone, Copy, serde::Serialize)]
pub(crate) struct AuditDifference {
    hour: i32,
    country: u32,
    group: u32,
    kind: u8,
    date: i32,
}

static LAST_SEALED_MONTH_DIFFERENCE: Mutex<Option<AuditDifference>> = Mutex::new(None);

pub(crate) fn last_sealed_month_difference() -> Option<AuditDifference> {
    *lock(&LAST_SEALED_MONTH_DIFFERENCE)
}

impl Store {
    #[cfg(test)]
    fn last_read(&self, country: u32) -> i32 {
        self.countries
            .binary_search_by_key(&country, |read| read.country)
            .map(|index| self.countries[index].last_whole)
            .unwrap_or(NEVER)
    }

    fn country(&mut self, country: u32) -> Result<&mut CountryRead, &'static str> {
        let index = match self
            .countries
            .binary_search_by_key(&country, |read| read.country)
        {
            Ok(i) => i,
            Err(i) => {
                allocation::reserve(&mut self.countries, 1)
                    .map_err(|_| "combat log country history allocation failed")?;
                self.countries.insert(
                    i,
                    CountryRead {
                        country,
                        last_whole: NEVER,
                        visited: NEVER,
                        sealed: native::SealedMonths::default(),
                    },
                );
                i
            }
        };
        Ok(&mut self.countries[index])
    }
}

const EMPTY_STORE: Store = Store {
    retained: crate::history_capture::State::new(),
    signal: None,
    hour: None,
    finals: Vec::new(),
    countries: Vec::new(),
};

#[cfg(not(any(test, feature = "test-support")))]
static STORE: Mutex<Store> = Mutex::new(EMPTY_STORE);
#[cfg(not(any(test, feature = "test-support")))]
static FORGET: AtomicBool = AtomicBool::new(false);

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static STORE: Mutex<Store> = const { Mutex::new(EMPTY_STORE) };
    static FORGET: AtomicBool = const { AtomicBool::new(false) };
    static DRAINS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(not(any(test, feature = "test-support")))]
fn with_store<R>(visit: impl FnOnce(&mut Store) -> R) -> R {
    visit(&mut lock(&STORE))
}
#[cfg(any(test, feature = "test-support"))]
fn with_store<R>(visit: impl FnOnce(&mut Store) -> R) -> R {
    STORE.with(|store| visit(&mut lock(store)))
}

#[cfg(not(any(test, feature = "test-support")))]
fn take_forget() -> bool {
    FORGET.swap(false, Ordering::AcqRel)
}
#[cfg(any(test, feature = "test-support"))]
fn take_forget() -> bool {
    FORGET.with(|forget| forget.swap(false, Ordering::AcqRel))
}

#[cfg(not(any(test, feature = "test-support")))]
pub fn forget_reads() {
    FORGET.store(true, Ordering::Release);
}
#[cfg(any(test, feature = "test-support"))]
pub fn forget_reads() {
    FORGET.with(|forget| forget.store(true, Ordering::Release));
}

/// # Safety
/// Caller holds hourly capture admission (or test lock) as the sole prune-log consumer while prune
/// storage stays live.
unsafe fn drain_prune_log(signal: i64, output: &mut Vec<native::Row>) -> bool {
    #[cfg(any(test, feature = "test-support"))]
    if !DRAINS.get() {
        return false;
    }
    // SAFETY: Caller holds hourly capture admission as the sole prune-log consumer.
    unsafe { crate::combat_log_prune::drain(signal, output) }.is_ok()
}

#[cfg(any(test, feature = "test-support"))]
pub fn drain_prune_log_for_test() {
    DRAINS.set(true);
    with_store(|store| *store = EMPTY_STORE);
    take_forget();
}

fn needs_all(last: i32, hour: i32) -> bool {
    last == NEVER || last > hour || month_bucket(last) < month_bucket(hour).saturating_sub(1)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    lend_lease: usize,
}

collector_primitives::capacity_max!(Capacities { rows, lend_lease });
impl Capacities {
    /// # Safety
    /// Run only during attach or the active session's serial sizing pass against the authenticated
    /// session image, while combat-log observer storage remains live and teardown is excluded.
    pub unsafe fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let writers =
            engine_land_inputs::combat_log_events::noncombat_writer_demand(memory, base, state);
        if let Ok(demand) = writers {
            crate::history_events::prepare_noncombat(demand);
        }
        let logs = native::row_demand(memory, base, state);
        let lend = lend_lease::row_demand(memory, base, state);
        if let Ok(rows) = logs {
            crate::country_scale::raise(&mut self.rows, rows);
            crate::combat_log_prune::prepare(rows);
        }
        if let Ok(rows) = lend {
            crate::country_scale::raise(&mut self.lend_lease, rows);
        }
        logs.and(lend).and(writers).map(|_| ())
    }
}

pub struct Captured {
    retained: crate::history_capture::Captured,
    rows: CaptureRows<native::Row>,
    lend_lease: CaptureRows<lend_lease::Row>,
    viewing: Result<i32, &'static str>,
    prune_observed: bool,
    selection: SliceSelection,
    memo: native::VariantMemo,
    signal: i64,
    drained: Vec<native::Row>,
}

const VIEWING_UNREAD: &str = "viewing country unreadable";

impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.lend_lease
            .fail(super::failed_capture(TABLES[2], hour, force_full, reason));
        self.retained.fail(reason);
        self.viewing = Err(reason);
    }
    pub fn vacant() -> Self {
        Self {
            retained: crate::history_capture::Captured::new(),
            rows: CaptureRows::new(0),
            lend_lease: CaptureRows::new(0),
            viewing: Err(VIEWING_UNREAD),
            prune_observed: false,
            selection: SliceSelection::new(),
            memo: native::VariantMemo::default(),
            signal: 0,
            drained: Vec::new(),
        }
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            retained: crate::history_capture::Captured::new(),
            rows: CaptureRows::new(capacity.rows),
            lend_lease: CaptureRows::new(capacity.lend_lease),
            viewing: Err(VIEWING_UNREAD),
            prune_observed: false,
            selection: SliceSelection::new(),
            memo: native::VariantMemo::default(),
            signal: 0,
            drained: Vec::new(),
        }
    }
    pub fn reset(&mut self) {
        self.retained
            .reset("combat log history acquisition pending");
        self.rows.reset();
        self.lend_lease.reset();
        self.viewing = Err(VIEWING_UNREAD);
        self.prune_observed = false;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.lend_lease.allocated_bytes()
            + self.memo.allocated_bytes()
            + self.drained.capacity() * size_of::<native::Row>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        self.lend_lease.grow(&mut capacity.lend_lease);
    }

    pub fn selection(&self) -> &SliceSelection {
        &self.selection
    }

    pub fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.selection.reset(hour, force_full);
    }

    pub fn set_signal(&mut self, signal: i64) {
        self.signal = signal;
    }

    /// # Safety
    /// Caller holds hourly capture admission for the active session (or exclusive fixture control)
    /// while the history-events lane stays live and teardown is excluded.
    pub unsafe fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.reset();
        self.selection.reset(hour, force_full);
        let (state_cost, log_cost) = with_store(|store| {
            let state_cost = {
                let _reads = crate::read_accounting::table(TABLES[1]);
                let started = qpc();
                self.viewing = native::viewing_country(memory, game_state).ok_or(VIEWING_UNREAD);
                // SAFETY: Caller holds hourly capture admission as the sole combat-log consumer.
                self.prune_observed = unsafe { self.begin(store, hour, force_full) };
                qpc().saturating_sub(started).max(0) as u64
            };
            let log_cost = {
                let _reads = crate::read_accounting::table(TABLES[0]);
                let started = qpc();
                store.retained.acquire(memory, base, game_state);
                store.retained.snapshot(&mut self.retained);
                self.rows.outcome =
                    self.read_log(store, memory, base, game_state, hour, force_full);
                qpc().saturating_sub(started).max(0) as u64
            };
            (state_cost, log_cost)
        });
        let lend_lease_cost = if should_walk(TABLES[2], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[2]);
            let started = qpc();
            self.lend_lease.outcome =
                lend_lease::walk(memory, base, game_state, &mut self.lend_lease.rows);
            qpc().saturating_sub(started).max(0) as u64
        } else {
            self.lend_lease.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
            0
        };
        let mut costs = [0; N];
        costs[..3].copy_from_slice(&[log_cost, state_cost, lend_lease_cost]);
        costs
    }

    /// # Safety
    /// Caller holds hourly capture admission as the sole combat-log consumer while prune storage
    /// stays live.
    unsafe fn begin(&mut self, store: &mut Store, hour: i32, force_full: bool) -> bool {
        let new_session =
            store.signal != Some(self.signal) || store.hour.is_some_and(|last| hour < last);
        let discontinuity =
            new_session || store.hour.and_then(|last| last.checked_add(1)) != Some(hour);
        if new_session {
            store.signal = Some(self.signal);
            store.finals.clear();
        }
        store.hour = Some(hour);
        let forget = take_forget();
        store
            .retained
            .begin(self.signal, force_full || forget || new_session);
        if forget || discontinuity {
            store.countries.clear();
        }
        if forget || force_full {
            for kept in &mut store.finals {
                kept.published = None;
            }
        }
        store.finals.retain(|kept| {
            kept.published.is_none_or(|published| {
                (published..published.saturating_add(RETAIN_HOURS)).contains(&hour)
            })
        });
        self.drained.clear();
        // SAFETY: Caller holds hourly capture admission as the sole prune-log consumer.
        let observed = unsafe { drain_prune_log(self.signal, &mut self.drained) };
        if allocation::reserve(&mut store.finals, self.drained.len()).is_err() {
            return false;
        }
        store.finals.extend(self.drained.iter().map(|&row| Final {
            row,
            published: None,
        }));
        observed
    }

    fn read_log(
        &mut self,
        store: &mut Store,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> Outcome {
        for kept in store.finals.iter().filter(|kept| kept.published.is_none()) {
            self.selection.select(kept.row.country);
        }
        if !force_full && hour.rem_euclid(STAGGER_PERIOD) == PRUNE_HOUR {
            let selection = &mut self.selection;
            if let Err(reason) =
                native::countries_with_deleted_groups(memory, base, game_state, |country| {
                    selection.select(country);
                })
            {
                return Outcome::Unavailable { reason };
            }
        }
        if crate::allocation::failed() {
            return Outcome::Unavailable {
                reason: "collector allocation failed",
            };
        }
        let selection = &self.selection;
        let outcome = native::walk_prepared(
            memory,
            base,
            game_state,
            store,
            |store, country| {
                if !selection.contains(country) {
                    return Ok(None);
                }
                let read = store.country(country)?;
                read.visited = hour;
                let whole = force_full || needs_all(read.last_whole, hour);
                if whole {
                    read.last_whole = hour;
                }
                Ok(Some(native::Months::Retaining {
                    whole,
                    sealed: &mut read.sealed,
                }))
            },
            &mut self.rows.rows,
            &mut self.memo,
        );
        store.countries.retain_mut(|read| {
            if let Some((group, kind, date)) = read.sealed.audit_difference.take() {
                *lock(&LAST_SEALED_MONTH_DIFFERENCE) = Some(AuditDifference {
                    hour,
                    country: read.country,
                    group,
                    kind,
                    date,
                });
            }
            read.visited == hour || !selection.contains(read.country)
        });
        let Outcome::Rows { len } = outcome else {
            store.countries.clear();
            return outcome;
        };
        let pending = store
            .finals
            .iter()
            .filter(|kept| kept.published.is_none())
            .count();
        let Some(required) = len.checked_add(pending) else {
            return Outcome::Unavailable {
                reason: "combat log final row demand overflow",
            };
        };
        if let Err(reason) =
            engine_native_memory::walk::Buffer::Growing(&mut self.rows.rows).ensure(required)
        {
            return Outcome::Unavailable { reason };
        }
        let mut at = len;
        for kept in store
            .finals
            .iter_mut()
            .filter(|kept| kept.published.is_none())
        {
            self.rows.rows[at] = kept.row;
            kept.published = Some(hour);
            at += 1;
        }
        Outcome::Rows { len: at }
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        let ordinary = [
            self.rows.outcome,
            match self.viewing {
                Ok(_) => Outcome::Rows { len: 1 },
                Err(reason) => Outcome::Unavailable { reason },
            },
            self.lend_lease.outcome,
        ];
        let retained = self.retained.outcomes();
        std::array::from_fn(|i| if i < 3 { ordinary[i] } else { retained[i - 3] })
    }

    fn ordinary_tables(&self) -> [TableView; 3] {
        let state = self.viewing.map(|viewing_country_raw| CombatLogStateRow {
            viewing_country_raw,
            prune_observed: u8::from(self.prune_observed),
        });
        [
            table::<tables::CombatLogEquipment, _>(self.rows.outcome, &self.rows.rows, |row| {
                CombatLogEquipmentRow {
                    country: row.country,
                    group: row.group,
                    kind: row.kind,
                    month: row.month,
                    variant: row.variant,
                    date: row.date,
                    amount_raw: row.amount_raw,
                }
            }),
            match state {
                Ok(row) => table::<tables::CombatLogState, _>(
                    Outcome::Rows { len: 1 },
                    std::slice::from_ref(&row),
                    Clone::clone,
                ),
                Err(reason) => table::<tables::CombatLogState, _>(
                    Outcome::Unavailable { reason },
                    &[] as &[CombatLogStateRow],
                    Clone::clone,
                ),
            },
            table::<tables::LendLeaseEquipment, _>(
                self.lend_lease.outcome,
                &self.lend_lease.rows,
                |row| LendLeaseEquipmentRow {
                    giver: row.giver,
                    exchange: row.exchange,
                    kind: row.kind,
                    variant: row.variant,
                    receiver: row.receiver,
                    amount_raw: row.amount_raw,
                },
            ),
        ]
    }
    pub fn table_inputs(&self) -> [replay_frame::frame::TableInput; N] {
        let mut ordinary = self.ordinary_tables().into_iter();
        let mut retained = self.retained.inputs().into_iter();
        std::array::from_fn(|i| {
            if i < 3 {
                replay_frame::frame::TableInput::Sampled(ordinary.next().unwrap())
            } else {
                retained.next().unwrap()
            }
        })
    }
    pub fn tables(&self) -> [TableView; N] {
        let mut ordinary = self.ordinary_tables().into_iter();
        let mut retained = self.retained.tables().into_iter();
        std::array::from_fn(|i| {
            if i < 3 {
                ordinary.next().unwrap()
            } else {
                retained.next().unwrap()
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    fn game_state(viewing: i32) -> Vec<u8> {
        let mut bytes = vec![0; 0x1000];
        bytes[engine_native::cgame_state::LOCAL_PLAYER_COUNTRY
            ..engine_native::cgame_state::LOCAL_PLAYER_COUNTRY + 4]
            .copy_from_slice(&viewing.to_le_bytes());
        bytes
    }

    fn row(country: u32, group: u32) -> native::Row {
        native::Row {
            country,
            group,
            kind: 0,
            month: 23_325,
            variant: 1326,
            date: 60_827_868,
            amount_raw: 300_000,
        }
    }

    #[test]
    fn the_state_is_hourly_and_lend_lease_daily_in_the_last_slot() {
        let _lock = crate::combat_log_prune::test_lock();
        crate::combat_log_prune::observe_for_test(true);
        drain_prune_log_for_test();
        let bytes = game_state(4);
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        for hour in 0..24 {
            // SAFETY: Test holds `TEST_LOCK` as the sole combat-log capture caller.
            unsafe { captured.capture(&memory, 0, 0x1000, hour, false) };
            let [log, state, lend_lease, ..] = captured.outcomes();
            assert_eq!(state, Outcome::Rows { len: 1 }, "{hour}");
            let skipped = lend_lease
                == Outcome::Unavailable {
                    reason: SKIPPED_REASON,
                };
            assert_eq!(skipped, hour != PRUNE_HOUR, "{hour}");
            assert!(
                matches!(log, Outcome::Unavailable { reason } if reason != SKIPPED_REASON),
                "{hour}"
            );
        }
        let [
            _,
            TableView::Complete(TableRows::CombatLogState(rows)),
            _,
            ..,
        ] = captured.tables()
        else {
            panic!("complete state");
        };
        assert_eq!(
            rows,
            [CombatLogStateRow {
                viewing_country_raw: 4,
                prune_observed: 1
            }]
        );
    }

    #[test]
    fn an_unobserved_prune_is_said_and_an_unreadable_state_is_unavailable() {
        let _lock = crate::combat_log_prune::test_lock();
        crate::combat_log_prune::observe_for_test(false);
        drain_prune_log_for_test();
        let bytes = game_state(-1);
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        // SAFETY: Test holds `TEST_LOCK` as the sole combat-log capture caller.
        unsafe { captured.capture(&memory, 0, 0x1000, 5, false) };
        let [
            _,
            TableView::Complete(TableRows::CombatLogState(rows)),
            _,
            ..,
        ] = captured.tables()
        else {
            panic!("complete state");
        };
        assert_eq!(
            rows,
            [CombatLogStateRow {
                viewing_country_raw: -1,
                prune_observed: 0
            }]
        );
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x10_0000,
            bytes: &bytes,
        };
        // SAFETY: Test holds `TEST_LOCK` as the sole combat-log capture caller.
        unsafe { captured.capture(&memory, 0, 0x1000, 5, false) };
        assert!(matches!(
            captured.tables()[1],
            TableView::Unavailable { .. }
        ));
    }

    const SIGNAL: i64 = 1_778_888_888;

    fn logged_state() -> Vec<u8> {
        let mut bytes = game_state(4);
        let header = engine_native::cgame_state::COMBAT_LOGS;
        bytes[header..header + 8].copy_from_slice(&0x1f00_u64.to_le_bytes());
        bytes[header + 8..header + 12].copy_from_slice(&1_u32.to_le_bytes());
        bytes[header + 12..header + 16].copy_from_slice(&1_u32.to_le_bytes());
        bytes
    }

    #[test]
    fn country_history_follows_selected_managers() {
        use engine_land_inputs::ncombat_log_corders_group_logs as groups;

        let mut bytes = logged_state();
        bytes.resize(0x6000, 0);
        let header = engine_native::cgame_state::COMBAT_LOGS;
        bytes[header..header + 8].copy_from_slice(&0x2000_u64.to_le_bytes());
        bytes[header + 8..header + 12].copy_from_slice(&41_u32.to_le_bytes());
        bytes[header + 12..header + 16].copy_from_slice(&41_u32.to_le_bytes());
        for country in 1..=40_usize {
            let manager = 0x3000 + country * native::SIZE;
            let pointer = 0x2000 - 0x1000 + country * 8;
            bytes[pointer..pointer + 8].copy_from_slice(&(manager as u64).to_le_bytes());
            let at = manager - 0x1000;
            bytes[at..at + 8].copy_from_slice(&(native::VTABLE_RVA.0 as u64).to_le_bytes());
            bytes[at + native::GROUPS..at + native::GROUPS + 8]
                .copy_from_slice(&0x4000_u64.to_le_bytes());
            bytes[at + native::GROUPS + 8..at + native::GROUPS + 12]
                .copy_from_slice(&1_u32.to_le_bytes());
            bytes[at + native::GROUPS + 12..at + native::GROUPS + 16]
                .copy_from_slice(&1_u32.to_le_bytes());
            bytes[at + native::COUNTRY..at + native::COUNTRY + 4]
                .copy_from_slice(&(country as u32).to_le_bytes());
        }
        bytes[0x3000..0x3008].copy_from_slice(&0x5000_u64.to_le_bytes());
        bytes[0x4000..0x4008].copy_from_slice(&(groups::VTABLE_RVA.0 as u64).to_le_bytes());
        let reference = (7_u64 << 32) | u64::from(engine_native::corders_group::REFERENCE_TYPE);
        bytes[0x4000 + groups::GROUP..0x4000 + groups::GROUP + 8]
            .copy_from_slice(&reference.to_le_bytes());
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut captured = Captured::with_capacities(&Capacities {
            rows: 0,
            lend_lease: 0,
        });
        let before = captured.allocated_bytes();
        let mut original_visited = Vec::new();
        let mut memo = native::VariantMemo::default();
        for (hour, force_full) in [(5, false), (6, false), (7, true), (8, false)] {
            captured.set_signal(SIGNAL);
            // SAFETY: Test holds `TEST_LOCK` as the sole combat-log capture caller.
            unsafe { captured.capture(&memory, 0, 0x1000, hour, force_full) };
            original_visited.clear();
            let outcome = native::walk(
                &memory,
                0,
                0x1000,
                |country| {
                    if !captured.selection.contains(country) {
                        return None;
                    }
                    original_visited.push(country);
                    Some(native::Months::All)
                },
                &mut Vec::new(),
                &mut memo,
            );
            assert_eq!(outcome, Outcome::Rows { len: 0 });
            assert_eq!(captured.rows.outcome, outcome);
            let visited = with_store(|store| {
                store
                    .countries
                    .iter()
                    .filter(|read| read.visited == hour)
                    .map(|read| read.country)
                    .collect::<Vec<_>>()
            });
            assert_eq!(visited, original_visited);
            assert_eq!(captured.allocated_bytes(), before);
            if !force_full {
                assert!(visited.len() < 40);
            }
        }
    }

    fn capture(
        captured: &mut Captured,
        bytes: &[u8],
        signal: i64,
        hour: i32,
        force_full: bool,
    ) -> Vec<native::Row> {
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes,
        };
        capture_at(captured, &memory, 0, 0x1000, signal, hour, force_full);
        let Outcome::Rows { len } = captured.rows.outcome else {
            panic!("complete read at {hour}: {:?}", captured.rows.outcome);
        };
        captured.rows.rows[..len].to_vec()
    }

    fn capture_at(
        captured: &mut Captured,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        signal: i64,
        hour: i32,
        force_full: bool,
    ) {
        captured.set_signal(signal);
        // SAFETY: Test holds `TEST_LOCK` as the sole combat-log capture caller.
        unsafe { captured.capture(memory, base, game_state, hour, force_full) };
    }

    fn buffers() -> [Captured; 4] {
        std::array::from_fn(|_| Captured::with_capacities(&Capacities::default()))
    }

    #[test]
    fn finals_of_other_countries_are_captured_with_every_gate_operand() {
        let _lock = crate::combat_log_prune::test_lock();
        crate::combat_log_prune::observe_for_test(true);
        drain_prune_log_for_test();
        let [mut captured, ..] = buffers();
        for (index, viewing) in [4_i32, 0, -1].into_iter().enumerate() {
            let mut bytes = logged_state();
            let offset = engine_native::cgame_state::LOCAL_PLAYER_COUNTRY;
            bytes[offset..offset + 4].copy_from_slice(&viewing.to_le_bytes());
            let signal = SIGNAL + index as i64;
            crate::combat_log_prune::append_for_test(signal, &[row(29, 7), row(30, 8)]);
            assert_eq!(
                capture(&mut captured, &bytes, signal, 5, true),
                [row(29, 7), row(30, 8)]
            );
        }
    }

    #[test]
    fn a_final_goes_out_once_with_whichever_buffer_reads_next_and_retires_a_day_later() {
        let _lock = crate::combat_log_prune::test_lock();
        crate::combat_log_prune::observe_for_test(true);
        drain_prune_log_for_test();
        let bytes = logged_state();
        let [mut a, mut b, mut c, mut d] = buffers();
        crate::combat_log_prune::append_for_test(SIGNAL, &[row(29, 7)]);
        assert_eq!(capture(&mut a, &bytes, SIGNAL, 5, false), [row(29, 7)]);
        assert!(a.selection().contains(29));
        assert!(capture(&mut b, &bytes, SIGNAL, 6, false).is_empty());
        assert!(!b.selection().contains(29));
        forget_reads();
        assert_eq!(capture(&mut c, &bytes, SIGNAL, 7, false), [row(29, 7)]);
        assert!(capture(&mut d, &bytes, SIGNAL, 8, false).is_empty());
        assert!(capture(&mut a, &bytes, SIGNAL, 7 + RETAIN_HOURS, false).is_empty());
        forget_reads();
        assert!(capture(&mut b, &bytes, SIGNAL, 8 + RETAIN_HOURS, false).is_empty());
    }

    #[test]
    fn a_load_leaves_no_final_of_the_left_branch_in_any_buffer() {
        let _lock = crate::combat_log_prune::test_lock();
        crate::combat_log_prune::observe_for_test(true);
        drain_prune_log_for_test();
        let bytes = logged_state();
        let [mut a, mut b, mut c, mut d] = buffers();
        crate::combat_log_prune::append_for_test(SIGNAL, &[row(29, 7)]);
        assert_eq!(capture(&mut a, &bytes, SIGNAL, 100, false), [row(29, 7)]);
        crate::combat_log_prune::append_for_test(SIGNAL, &[row(30, 8)]);
        let loaded = SIGNAL + 1;
        assert!(capture(&mut b, &bytes, loaded, 50, true).is_empty());
        assert!(capture(&mut c, &bytes, loaded, 51, false).is_empty());
        assert!(capture(&mut d, &bytes, loaded, 52, false).is_empty());
        assert!(capture(&mut a, &bytes, loaded, 53, false).is_empty());
        forget_reads();
        assert!(capture(&mut b, &bytes, loaded, 54, true).is_empty());
        crate::combat_log_prune::append_for_test(loaded, &[row(31, 9)]);
        assert_eq!(capture(&mut c, &bytes, loaded, 60, false), [row(31, 9)]);
        assert!(capture(&mut d, &bytes, loaded, 40, true).is_empty());
    }

    #[test]
    fn a_full_hour_in_the_same_session_publishes_kept_finals_again() {
        let _lock = crate::combat_log_prune::test_lock();
        crate::combat_log_prune::observe_for_test(true);
        drain_prune_log_for_test();
        let bytes = logged_state();
        let [mut a, mut b, ..] = buffers();
        crate::combat_log_prune::append_for_test(SIGNAL, &[row(29, 7)]);
        assert_eq!(capture(&mut a, &bytes, SIGNAL, 5, false), [row(29, 7)]);
        assert_eq!(capture(&mut b, &bytes, SIGNAL, 9, true), [row(29, 7)]);
        forget_reads();
        assert_eq!(capture(&mut a, &bytes, SIGNAL, 12, false), [row(29, 7)]);
    }

    fn day(year: i32, day: i32) -> i32 {
        43_800_000 + year * 8760 + day * 24
    }

    mod equipment_month_tests;

    #[test]
    fn a_country_is_read_whole_unless_read_this_or_last_month() {
        let february = day(1941, 40);
        assert!(!needs_all(day(1941, 10), february), "January");
        assert!(!needs_all(day(1941, 35), february), "February");
        assert!(needs_all(day(1940, 364), february), "December");
        assert!(!needs_all(day(1940, 364), day(1941, 0)), "across a year");
        assert!(needs_all(day(1941, 10), day(1941, 63)), "January in March");
        assert!(needs_all(NEVER, february));
        assert!(needs_all(february + 1, february), "a later read");
    }

    #[test]
    fn unpublished_reads_and_loads_forget_every_last_read() {
        let _lock = crate::combat_log_prune::test_lock();
        crate::combat_log_prune::observe_for_test(true);
        drain_prune_log_for_test();
        let bytes = logged_state();
        let [mut a, ..] = buffers();
        capture(&mut a, &bytes, SIGNAL, 5, false);
        let last = |country| with_store(|store| store.last_read(country));
        with_store(|store| store.country(29).unwrap().last_whole = 5);
        capture(&mut a, &bytes, SIGNAL, 6, false);
        assert_eq!(last(29), 5, "kept while nothing went unpublished");
        forget_reads();
        capture(&mut a, &bytes, SIGNAL, 7, false);
        assert_eq!(last(29), NEVER);
        with_store(|store| store.country(29).unwrap().last_whole = 7);
        capture(&mut a, &bytes, SIGNAL + 1, 8, true);
        assert_eq!(last(29), NEVER, "a load");
    }

    #[test]
    fn conversion_keeps_native_values() {
        let mut captured = Captured::with_capacities(&Capacities {
            rows: 2,
            lend_lease: 1,
        });
        captured.rows.rows[0] = row(4, 12793);
        captured.rows.rows[1] = native::Row {
            kind: native::RECOVERED_KIND,
            variant: 1846,
            amount_raw: i64::MAX,
            ..row(4, 12793)
        };
        captured.rows.outcome = Outcome::Rows { len: 2 };
        captured.lend_lease.rows[0] = lend_lease::Row {
            giver: 4,
            exchange: 97730,
            kind: lend_lease::SUNK_KIND,
            variant: 5157,
            receiver: 9,
            amount_raw: 300_000,
        };
        captured.lend_lease.outcome = Outcome::Rows { len: 1 };
        captured.viewing = Ok(4);
        let [
            TableView::Complete(TableRows::CombatLogEquipment(rows)),
            TableView::Complete(TableRows::CombatLogState(state)),
            TableView::Complete(TableRows::LendLeaseEquipment(lend)),
            ..,
        ] = captured.tables()
        else {
            panic!("complete tables");
        };
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[1].kind, rows[1].amount_raw), (4, i64::MAX));
        assert_eq!(
            state,
            [CombatLogStateRow {
                viewing_country_raw: 4,
                prune_observed: 0
            }]
        );
        assert_eq!(
            lend,
            [LendLeaseEquipmentRow {
                giver: 4,
                exchange: 97730,
                kind: 1,
                variant: 5157,
                receiver: 9,
                amount_raw: 300_000,
            }]
        );
        captured.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 9,
        };
        assert!(matches!(captured.tables()[0], TableView::Truncated { .. }));
    }
}
