//! Numerical holder for the one original hourly naval mission pass.
//! This helper reads no live memory and holds no pointers. The existing naval
//! source owner alone borrows, materializes and retires original controls.
use crate::table_capture::Arena;
use replay::rows::{
    TransportLedger, TransportMissionInputs, TransportMissionReceiver, TransportNavy,
};

use replay::transport_mission_view::{RISK_REFUSAL_NAMES as RISK_NAMES, risk_refusals};
use std::sync::atomic::{AtomicU64, Ordering};

static REFUSALS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];
static LEDGER_REASONS: crate::status::RefusalReasons = crate::status::RefusalReasons::new();
static ACCESS_REFUSALS: AtomicU64 = AtomicU64::new(0);
static RISK_REFUSALS: [AtomicU64; RISK_NAMES.len()] =
    [const { AtomicU64::new(0) }; RISK_NAMES.len()];
fn refuse(index: usize) {
    REFUSALS[index].fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn status() -> impl serde::Serialize {
    const NAMES: [&str; 10] = [
        "transport original physical navy domain refused",
        "transport original outer ledger refused",
        "transport original nullable ledger or receiver certificate refused",
        "transport receiver dictionary refused",
        "transport original logical owner refused",
        "transport original in-regions refused",
        "transport original signed combat count refused",
        "transport original received supply refused",
        "transport combined NAVAL_DETECTION refused",
        "transport original spotting cache pair refused",
    ];
    crate::status::status!({ "refusals": crate::status::map(NAMES.into_iter().zip(REFUSALS.iter().map(|count| count.load(Ordering::Relaxed)))),
        "transport original navy access modes refused": ACCESS_REFUSALS.load(Ordering::Relaxed),
        "transport original naval search graph refused": super::transport_naval_search::refusals(),
        "spotting_context_refusals": super::transport_spotting_context::status(),
        "air_refusals": super::air::transport_status(),
        "nullable_ledger_or_receiver_certificate_reasons": &LEDGER_REASONS,
        "risk_ship_refusals": super::transport_risk_ships::status(),
        "risk_refusals": crate::status::map(RISK_NAMES.into_iter().zip(RISK_REFUSALS.iter().map(|count| count.load(Ordering::Relaxed)))),
        "scope": "cumulative current-hour original transport input refusals; contribution and own-risk demands retain independent failures" })
}

type Fact<T> = Result<T, &'static str>;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Capacities {
    pub navies: usize,
    pub ledgers: usize,
    pub occurrences: usize,
    pub receivers: usize,
    pub access_modes: usize,
    pub search: engine::transport_naval_search::Demand,
    pub ships: usize,
    pub weather_entries: usize,
}

#[derive(Default)]
struct Navy {
    slot: u32,
    present: bool,
    regions: Option<u32>,
    first: usize,
    count: usize,
    reason: Option<&'static str>,
    access: Option<(usize, usize)>,
}

#[derive(Default)]
struct Ledger {
    region: u32,
    first: usize,
    count: usize,
    reason: Option<&'static str>,
}

pub(crate) struct Shared {
    hour: Option<i32>,
    physical_count: Fact<usize>,
    navies: Arena<Navy>,
    ledgers: Arena<Ledger>,
    occurrences: Arena<u64>,
    receivers: Arena<TransportMissionReceiver>,
    access_modes: Arena<u8>,
    navy_reason: Option<&'static str>,
    receiver_reason: Option<&'static str>,
    current_navy: Option<usize>,
    finished: bool,
    growing: bool,
    naval_search: super::transport_naval_search::Captured,
    spotting_regions: super::transport_spotting_context::Captured,
    risk_ships: super::transport_risk_ships::Captured,
}

impl Shared {
    pub(crate) fn new(capacity: Capacities) -> Self {
        let mut captured = Self {
            hour: None,
            physical_count: Err("transport physical manager not supplied"),
            navies: Arena::new(capacity.navies),
            ledgers: Arena::new(capacity.ledgers),
            occurrences: Arena::new(capacity.occurrences),
            receivers: Arena::new(capacity.receivers),
            access_modes: Arena::new(capacity.access_modes),
            navy_reason: Some("transport physical manager not supplied"),
            receiver_reason: Some("transport mission receivers not supplied"),
            current_navy: None,
            finished: false,
            growing: false,
            naval_search: super::transport_naval_search::Captured::new(
                capacity.search.regions,
                capacity.search.edges,
            ),
            spotting_regions: super::transport_spotting_context::Captured::new(
                super::transport_spotting_context::Capacities {
                    regions: capacity.search.regions,
                    ..Default::default()
                },
            ),
            risk_ships: super::transport_risk_ships::Captured::new(
                capacity.receivers,
                capacity.ships,
            ),
        };
        captured
            .naval_search
            .prepare(capacity.search, capacity.weather_entries);
        captured
    }

    pub(crate) fn reset(&mut self) {
        self.reset_pass();
        self.risk_ships.reset();
    }

    fn reset_pass(&mut self) {
        self.hour = None;
        self.physical_count = Err("transport physical manager not supplied");
        self.navies.reset();
        self.ledgers.reset();
        self.occurrences.reset();
        self.receivers.reset();
        self.access_modes.reset();
        self.navy_reason = Some("transport physical manager not supplied");
        self.receiver_reason = Some("transport mission receivers not supplied");
        self.current_navy = None;
        self.finished = false;
        self.growing = false;
        self.naval_search.reset();
        self.spotting_regions.reset();
    }

    pub(crate) fn begin(&mut self, hour: i32, original_physical_count: Fact<usize>) {
        self.reset_pass();
        self.risk_ships.begin(hour);
        self.hour = Some(hour);
        self.spotting_regions.begin(hour, Ok(0));
        self.physical_count = original_physical_count;
        self.navy_reason = original_physical_count.err();
        self.receiver_reason = None;
        if let Ok(count) = original_physical_count {
            self.navies.required = count;
        } else {
            refuse(0);
        }
    }

    pub(crate) fn begin_growing(&mut self, hour: i32, original_physical_count: Fact<usize>) {
        self.begin(hour, original_physical_count);
        self.growing = true;
        if let Ok(count) = original_physical_count {
            crate::table_capture::grow_vec(&mut self.navies.first, count);
        }
    }

    /// Called in the original complete physical order. A known null/empty
    /// manager has count zero and invokes no per-slot observer.
    pub(crate) fn navy(&mut self, slot: u32, present: bool, original_regions: Fact<u32>) {
        self.current_navy = None;
        if self.finished || self.hour.is_none() {
            self.navy_reason
                .get_or_insert("transport navy supplied outside original pass");
            return;
        }
        let index = self.navies.len;
        if usize::try_from(slot).ok() != Some(index) {
            self.navy_reason
                .get_or_insert("transport physical navy order incomplete");
            return;
        }
        let Some(row) = self.navies.first.get_mut(index) else {
            self.navy_reason
                .get_or_insert("transport physical navy arena exhausted");
            return;
        };
        if !present && original_regions != Ok(0) {
            self.navy_reason
                .get_or_insert("transport null navy operands invalid");
            refuse(0);
        }
        *row = Navy {
            slot,
            present,
            regions: original_regions.ok(),
            first: self.ledgers.len,
            count: 0,
            reason: original_regions.err(),
            access: None,
        };
        self.current_navy = Some(index);
        self.navies.len += 1;
    }

    /// Borrow only the already authenticated original header. This copies
    /// numerical bytes during its native loan and retains no selector/root.
    pub(crate) fn access_modes(
        &mut self,
        memory: &impl engine::Memory,
        original: Fact<engine::cpdx_array::Array>,
    ) {
        let result = original.and_then(|array| {
            let count = array.count as usize;
            self.access_modes.required = self.access_modes.required.saturating_add(count);
            let start = self.access_modes.len;
            let end = start
                .checked_add(count)
                .ok_or("transport original navy access extent overflow")?;
            if self.growing {
                engine::walk::Buffer::Growing(&mut self.access_modes.first).ensure(end)?;
            }
            let target = start
                .checked_add(count)
                .and_then(|end| self.access_modes.first.get_mut(start..end))
                .ok_or("transport original navy access arena exhausted")?;
            if count != 0 && !engine::read_slice(memory, array.data, target) {
                return Err("transport original navy access bytes unreadable");
            }
            self.access_modes.len += count;
            Ok((start, count))
        });
        if result.is_err() {
            ACCESS_REFUSALS.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(index) = self.current_navy {
            self.navies.first[index].access = result.ok();
        }
    }

    pub(crate) fn retain_access_demand(&mut self, required: usize) {
        self.access_modes.required = self.access_modes.required.max(required);
    }

    pub(crate) fn capture_naval_search_with_weather<M: engine::Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        weather: Result<&engine::cweather_manager::Reader<'_, M>, &'static str>,
    ) {
        self.naval_search.capture_with_spotting_weather(
            memory,
            base,
            state,
            &mut self.spotting_regions,
            weather,
        );
    }
    pub(crate) fn original_strict_weather<M: engine::Memory>(
        &mut self,
        memory: &M,
        base: usize,
        region: u32,
        weather: Result<&engine::cweather_manager::Reader<'_, M>, &'static str>,
    ) -> Result<[engine::modifier_accumulator::Value; 4], &'static str> {
        self.naval_search
            .strict_weather(memory, base, region, weather)
    }
    pub(crate) fn finish_weather_union(&mut self) {
        self.naval_search.retire_weather();
    }

    pub(crate) fn take_spotting_regions(
        &mut self,
        hour: i32,
    ) -> super::transport_spotting_context::RegionLoan {
        self.spotting_regions.take_regions(hour)
    }
    pub(crate) fn restore_spotting_regions(
        &mut self,
        loan: super::transport_spotting_context::RegionLoan,
    ) -> Result<(), &'static str> {
        self.spotting_regions.restore_regions(loan)
    }

    /// Entire original sparse outer-header failure; keep the known region
    /// count and all observed growth floors, but publish no ledger prefix.
    pub(crate) fn refuse_ledgers(&mut self, reason: &'static str) {
        refuse(1);
        if let Some(index) = self.current_navy {
            self.navies.first[index].reason.get_or_insert(reason);
        }
    }

    /// `required` is the original registration count even after an original
    /// read/certificate refusal. A partially copied payload is never supplied.
    pub(crate) fn ledger(&mut self, region: u32, required: usize, original: Fact<&[u64]>) {
        self.ledgers.required = self.ledgers.required.saturating_add(1);
        self.occurrences.required = self.occurrences.required.saturating_add(required);
        let Some(navy_index) = self.current_navy else {
            return;
        };
        if self.growing {
            crate::table_capture::grow_vec(
                &mut self.ledgers.first,
                self.ledgers.len.saturating_add(1),
            );
            crate::table_capture::grow_vec(
                &mut self.occurrences.first,
                self.occurrences.len.saturating_add(required),
            );
        }
        let navy = &mut self.navies.first[navy_index];
        if self.finished || !navy.present || navy.regions.is_none_or(|count| region >= count) {
            navy.reason
                .get_or_insert("transport original ledger region invalid");
            return;
        }
        if navy.count != 0 && self.ledgers.first[self.ledgers.len - 1].region >= region {
            navy.reason
                .get_or_insert("transport original ledger order invalid");
            return;
        }
        let Some(row) = self.ledgers.first.get_mut(self.ledgers.len) else {
            navy.reason
                .get_or_insert("transport ledger arena exhausted");
            return;
        };
        let mut reason = original.err();
        let first = self.occurrences.len;
        if let Ok(values) = original {
            if values.len() != required || required == 0 {
                reason = Some("transport original ledger count mismatch");
            } else if let Some(target) = first
                .checked_add(required)
                .and_then(|end| self.occurrences.first.get_mut(first..end))
            {
                target.copy_from_slice(values);
                self.occurrences.len += required;
            } else {
                reason = Some("transport occurrence arena exhausted");
            }
        }
        if let Some(reason) = reason {
            refuse(2);
            LEDGER_REASONS.record(reason);
        }
        *row = Ledger {
            region,
            first,
            count: required,
            reason,
        };
        self.ledgers.len += 1;
        navy.count += 1;
    }

    /// Exactly one materialized result per genuine full receiver certificate.
    /// Provider memoization, original failures and pointer retirement belong
    /// to the existing source owner, before this numerical-only handoff.
    pub(crate) fn receiver(&mut self, original: TransportMissionReceiver) {
        self.receivers.required = self.receivers.required.saturating_add(1);
        if self.finished || self.hour.is_none() {
            self.receiver_reason
                .get_or_insert("transport receiver supplied outside original pass");
            return;
        }
        if self.growing {
            crate::table_capture::grow_vec(
                &mut self.receivers.first,
                self.receivers.len.saturating_add(1),
            );
        }
        let Some(target) = self.receivers.first.get_mut(self.receivers.len) else {
            self.receiver_reason
                .get_or_insert("transport receiver dictionary arena exhausted");
            return;
        };
        for (index, missing) in [
            original.logical_owner.is_none(),
            original.in_regions.is_none(),
            original.in_regions.is_some_and(|flag| flag != 0)
                && original.combat_references.is_none(),
            original.received_supply_ratio.is_none(),
            original.in_regions.is_some_and(|flag| flag != 0)
                && original.combat_references.is_some_and(|count| count <= 0)
                && original.combined_detection.is_none(),
            original.in_regions.is_some_and(|flag| flag != 0)
                && original.combat_references.is_some_and(|count| count <= 0)
                && original.detection_cache.is_none(),
        ]
        .into_iter()
        .enumerate()
        {
            if missing {
                refuse(index + 4);
            }
        }
        for (counter, missing) in RISK_REFUSALS.iter().zip(risk_refusals(&original)) {
            if missing {
                counter.fetch_add(1, Ordering::Relaxed);
            }
        }
        *target = original;
        self.receivers.len += 1;
    }

    pub(crate) fn refuse_receivers(&mut self, reason: &'static str) {
        refuse(3);
        self.receiver_reason.get_or_insert(reason);
    }

    /// Called only after all same-pass original borrowers join. This does not
    /// retire native controls: that separate owner operation is mandatory.
    pub(crate) fn finish(&mut self) {
        self.current_navy = None;
        self.finished = true;
        if self.physical_count != Ok(self.navies.len) {
            refuse(0);
            self.navy_reason
                .get_or_insert("transport physical navy census incomplete");
        }
        if self
            .navies
            .first
            .first()
            .is_some_and(|row| row.slot == 0 && row.present)
            && self.navies.len != 0
        {
            refuse(0);
            self.navy_reason
                .get_or_insert("transport physical slot zero is not native null");
        }
        let receivers = &mut self.receivers.first[..self.receivers.len];
        receivers.sort_unstable_by_key(|receiver| receiver.task_force);
        if receivers
            .iter()
            .any(|receiver| receiver.task_force >> 32 == 0)
            || receivers
                .windows(2)
                .any(|pair| pair[0].task_force == pair[1].task_force)
        {
            refuse(3);
            self.receiver_reason
                .get_or_insert("transport receiver dictionary certificate invalid");
        }
    }

    /// Terminal failure retains every known arena floor; it never republishes
    /// earlier numerical rows or resets native counters/loans.
    pub(crate) fn fail(&mut self, reason: &'static str) {
        self.risk_ships.fail();
        self.navy_reason.get_or_insert(reason);
        self.receiver_reason.get_or_insert(reason);
        self.current_navy = None;
        self.finished = true;
        self.spotting_regions.fail();
        self.naval_search.retire_weather();
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.navies.allocated_bytes()
            + self.ledgers.allocated_bytes()
            + self.occurrences.allocated_bytes()
            + self.receivers.allocated_bytes()
            + self.access_modes.allocated_bytes()
            + self.naval_search.allocated_bytes()
            + self.risk_ships.allocated_bytes()
            + self.spotting_regions.allocated_bytes()
    }

    pub(crate) fn grow(&mut self, capacity: &mut Capacities) {
        self.risk_ships.prepare(capacity.receivers, capacity.ships);
        self.navies.grow(&mut capacity.navies);
        self.ledgers.grow(&mut capacity.ledgers);
        self.occurrences.grow(&mut capacity.occurrences);
        // Original receiver memo exhaustion can hide dictionary demand. Every
        // distinct receiver still has an occurrence in the same original pass.
        crate::table_capture::grow_capacity(&mut capacity.receivers, self.occurrences.required);
        self.receivers.grow(&mut capacity.receivers);
        self.access_modes.grow(&mut capacity.access_modes);
        self.naval_search
            .prepare(capacity.search, capacity.weather_entries);
        self.spotting_regions
            .prepare(super::transport_spotting_context::Capacities {
                regions: capacity.search.regions,
                ..Default::default()
            });
    }

    pub(crate) fn retain_required(&mut self, capacity: Capacities) {
        self.navies.required = self.navies.required.max(capacity.navies);
        self.ledgers.required = self.ledgers.required.max(capacity.ledgers);
        self.occurrences.required = self.occurrences.required.max(capacity.occurrences);
        self.receivers.required = self.receivers.required.max(capacity.receivers);
        self.access_modes.required = self.access_modes.required.max(capacity.access_modes);
    }

    pub(crate) fn required(&self) -> Capacities {
        Capacities {
            navies: self.navies.required,
            ledgers: self.ledgers.required,
            occurrences: self.occurrences.required,
            receivers: self.receivers.required,
            access_modes: self.access_modes.required,
            ..Default::default()
        }
    }

    /// Off-thread checked projection owns only numerical payload. Required
    /// naming-prefix/table admission remains the actual singleton writer's job.
    pub(crate) fn projection(&self) -> std::io::Result<Option<TransportMissionInputs>> {
        let Some(sample_hour) = self.hour else {
            return Ok(None);
        };
        if !self.finished {
            return Ok(None);
        }
        let navies = if self.navy_reason.is_none() {
            let mut values = engine::allocation::vec(self.navies.len)?;
            for navy in &self.navies.first[..self.navies.len] {
                let ledgers = if navy.reason.is_none() {
                    let mut values = engine::allocation::vec(navy.count)?;
                    for row in &self.ledgers.first[navy.first..navy.first + navy.count] {
                        values.push(TransportLedger {
                            region: row.region,
                            occurrences: if row.reason.is_none() {
                                Some(engine::allocation::copy_vec(
                                    &self.occurrences.first[row.first..row.first + row.count],
                                )?)
                            } else {
                                None
                            },
                        });
                    }
                    Some(values)
                } else {
                    None
                };
                values.push(TransportNavy {
                    slot: navy.slot,
                    present: navy.present,
                    region_slots: navy.regions,
                    ledgers,
                    access_modes: navy
                        .access
                        .map(|(start, count)| {
                            engine::allocation::copy_vec(
                                &self.access_modes.first[start..start + count],
                            )
                        })
                        .transpose()?,
                });
            }
            Some(values)
        } else {
            None
        };
        let receivers = if self.receiver_reason.is_none() {
            Some(engine::allocation::copy_vec(
                &self.receivers.first[..self.receivers.len],
            )?)
        } else {
            None
        };
        Ok(Some(TransportMissionInputs {
            sample_hour,
            navies,
            receivers,
            naval_search: self.naval_search.projection()?,
            spotting_context: self.spotting_regions.projection()?,
            air: None,
            risk_ships: self.risk_ships.packet(sample_hour)?,
        }))
    }

    pub(crate) fn prepare_transport_models(
        &mut self,
        hour: i32,
    ) -> &mut super::transport_risk_ships::Captured {
        self.risk_ships.begin_models(hour);
        &mut self.risk_ships
    }

    pub(crate) fn risk_ships_mut(&mut self) -> &mut super::transport_risk_ships::Captured {
        &mut self.risk_ships
    }
}

#[cfg(test)]
#[path = "transport_mission_shared_tests.rs"]
mod tests;
