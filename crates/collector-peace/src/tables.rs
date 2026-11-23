pub use crate::peace_settlements::Capacities;
use crate::peace_settlements::Storage;
use engine_native_memory::walk::Outcome;
use replay_data::{rows, rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;
pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::PeaceSettlements];
pub struct Captured {
    storage: Storage,
    drained: Result<u64, &'static str>,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            events: 0,
            countries: 0,
            actions: 0,
            transfers: 0,
            name_bytes: 0,
        })
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.storage.used = [0; 7];
        self.drained = Err(reason);
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            storage: Storage::with_capacities(*capacity),
            drained: Err("peace finalizer log not drained"),
        }
    }
    pub fn reset(&mut self) {
        self.storage.used = [0; 7];
        self.drained = Err("peace finalizer log not drained");
    }
    pub fn allocated_bytes(&self) -> usize {
        self.storage.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        let current = self.storage.capacity();
        let omitted = usize::try_from(self.omitted()).unwrap_or(usize::MAX);
        if omitted != 0 {
            *capacity = capacity.merge(Capacities {
                events: current.events.saturating_add(omitted),
                countries: current.countries.saturating_add(omitted),
                actions: current.actions.saturating_add(omitted),
                transfers: current.transfers.saturating_add(omitted),
                name_bytes: current.name_bytes.saturating_add(
                    self.storage.events[..self.storage.used[0]]
                        .iter()
                        .fold(0_usize, |total, event| {
                            total.saturating_add(event.snapshot.required_name_bytes)
                        }),
                ),
            });
        }
        if !self.storage.capacity().contains(*capacity)
            && let Some(storage) = Storage::new(*capacity)
        {
            self.storage = storage;
        }
    }
    pub fn drain(&mut self, continuous: bool) {
        self.drained = crate::peace_settlements::drain(&mut self.storage, continuous);
    }
    pub fn capture_bids(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
        state: usize,
        hour: i32,
    ) {
        self.reset();
        let attempt = (|| {
            let array = state
                .checked_add(engine_native::cgame_state::PEACE_CONFERENCES_DATA)
                .and_then(|at| engine_native_memory::cpdx_array::Array::read(memory, at, 8))
                .ok_or("peace bid roster unreadable or invalid")?;
            if self.storage.events.len() < array.count as usize {
                engine_allocation::resize_default(&mut self.storage.events, array.count as usize)
                    .map_err(|_| "peace bid event allocation failed")?;
            }
            for index in 0..array.count {
                let pointer = array
                    .element(index, 8)
                    .and_then(|at| engine_native_memory::read::<u64>(memory, at))
                    .ok_or("peace bid roster pointer unreadable")?
                    as usize;
                self.storage
                    .observe_bids(memory, base, pointer, hour, index as u64);
            }
            Ok(0)
        })();
        self.drained = attempt;
    }
    fn omitted(&self) -> u64 {
        self.storage.events[..self.storage.used[0]]
            .iter()
            .fold(self.drained.unwrap_or(0), |n, e| {
                n.saturating_add(e.snapshot.omitted as u64)
            })
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [match self.drained {
            Err(reason) => Outcome::Unavailable { reason },
            Ok(_) if self.omitted() != 0 => Outcome::Truncated {
                len: self.storage.used[0],
                unread_rows: self.omitted().min(u32::MAX as u64) as u32,
            },
            Ok(_) => Outcome::Rows {
                len: self.storage.used[0],
            },
        }]
    }
    pub fn tables(&self) -> [TableView; N] {
        if let Err(reason) = self.drained {
            return [TableView::Unavailable {
                reason: engine_allocation::string(reason).unwrap_or_default(),
            }];
        }
        let events = engine_allocation::owned_vec(self.convert());
        let rows = TableRows::PeaceSettlements(events);
        [if self.omitted() == 0 {
            TableView::Complete(rows)
        } else {
            TableView::Truncated {
                rows,
                missing: self.omitted().min(u32::MAX as u64) as u32,
            }
        }]
    }
    fn convert(&self) -> std::io::Result<Vec<rows::PeaceSettlementRow>> {
        use engine_allocation::{copy_vec, map_slice, vec};
        let storage = &self.storage;
        let mut events = vec(storage.used[0])?;
        for e in &storage.events[..storage.used[0]] {
            let s = e.snapshot;
            let mut actions = vec(s.actions)?;
            for a in &storage.actions[e.actions..e.actions + s.actions] {
                let o = a.operands;
                actions.push(rows::SettledPeaceAction {
                    country: a.country,
                    turn: a.turn,
                    ordinal: a.ordinal,
                    token: o.token,
                    taker: o.taker,
                    giver: o.giver,
                    negotiator: o.negotiator,
                    status: o.status,
                    cost: o.cost,
                    form: o.form as u8,
                    state: o.state,
                    flags: o.flags,
                    country_operand: o.country_operand,
                    puppet_auxiliary: o.puppet_auxiliary,
                    ship: o.ship,
                    ratio: o.ratio,
                    cached_name: Some(if a.name_len <= a.name_inline.len() {
                        copy_vec(&a.name_inline[..a.name_len])?
                    } else {
                        let start = e.name_bytes + a.name_start;
                        copy_vec(&storage.name_bytes[start..start + a.name_len])?
                    }),
                    name_key: o
                        .name_key
                        .map(|key| {
                            copy_vec(&key[..key.iter().position(|b| *b == 0).unwrap_or(key.len())])
                        })
                        .transpose()?,
                });
            }
            events.push(rows::PeaceSettlementRow {
                sequence: e.sequence,
                conference: s.id,
                hour: e.hour,
                winners: copy_vec(&storage.winners[e.winners..e.winners + s.winners])?,
                losers: copy_vec(&storage.losers[e.losers..e.losers + s.losers])?,
                actions,
                transfers: map_slice(
                    &storage.transfers[e.transfers..e.transfers + s.transfers],
                    |t| rows::SettledPeaceTransfer {
                        state: t.state,
                        taker: t.taker,
                        automatic: t.automatic,
                    },
                )?,
                subjects: map_slice(
                    &storage.subjects[e.subjects..e.subjects + s.subjects],
                    |s| rows::SettledPeaceSubject {
                        overlord: s.overlord,
                        subject: s.subject,
                    },
                )?,
                failure: s
                    .failure
                    .map(|reason| copy_vec(reason.as_bytes()))
                    .transpose()?,
                omitted: s.omitted as u64,
            });
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resident_bids_grow_all_original_headers_before_first_and_later_sample() {
        use crate::peace_settlement_tests::fixture::Fixture;
        let mut captured = Captured::vacant();
        for (countries, actions, name_len) in [(1_500, 8_193, 5_001), (5_000, 16_385, 10_003)] {
            let mut fixture = Fixture::new(countries, actions);
            fixture.bytes[0x100 + engine_peace::peace_conference::FINALIZED] = 0;
            let payload = fixture.bytes.len();
            fixture.bytes.resize(payload + name_len + 1, b'x');
            fixture.bytes[payload + name_len] = 0;
            let text = 0x20000 + engine_peace::peace_action::CACHED_NAME;
            fixture.u64(text, payload as u64);
            fixture.u64(text + engine_native_memory::msvc_string::LENGTH, name_len as u64);
            fixture.u64(text + engine_native_memory::msvc_string::CAPACITY, name_len as u64);
            let roster = fixture.bytes.len();
            fixture.bytes.resize(roster + 16, 0);
            fixture.array(
                0x10000 + engine_native::cgame_state::PEACE_CONFERENCES_DATA,
                roster,
                2,
                8,
            );
            fixture.u64(roster, 0x100);
            fixture.u64(roster + 8, 0x100);
            for hour in [91, 92] {
                captured.capture_bids(&fixture.memory(), 0, 0x10000, hour);
                assert_eq!(captured.outcomes(), [Outcome::Rows { len: 2 }]);
                let rows = captured.convert().unwrap();
                for row in rows {
                    assert_eq!(row.winners.len(), countries);
                    assert_eq!(row.actions.len(), actions);
                    assert_eq!(row.actions[0].cached_name.as_ref().unwrap().len(), name_len);
                    assert_eq!(row.omitted, 0);
                    assert_eq!(row.failure, None);
                }
            }
        }
    }

    #[test]
    fn ordinary_forms_capture_loaded_cached_names_without_invented_keys() {
        let _globals = crate::peace_settlements::test_globals();
        let mut fixture = crate::peace_settlement_tests::fixture::Fixture::new(1, 4);
        let forms = [
            (engine_peace::ctake_state_action::VTABLE_RVA, "Loaded state"),
            (
                engine_peace::cliberate_country_action::VTABLE_RVA,
                "Loaded release",
            ),
            (
                engine_peace::cpuppet_country_action::VTABLE_RVA,
                "Loaded puppet",
            ),
            (
                engine_peace::cforce_government_action::VTABLE_RVA,
                "Loaded rule",
            ),
        ];
        for (index, (vtable, name)) in forms.iter().enumerate() {
            let at = 0x20000 + index * 0x70;
            fixture.u64(at, vtable.0 as u64);
            let text = at + engine_peace::peace_action::CACHED_NAME;
            fixture.bytes[text..text + name.len()].copy_from_slice(name.as_bytes());
            fixture.u64(text + engine_native_memory::msvc_string::LENGTH, name.len() as u64);
        }
        let mut captured = Captured::with_capacities(&Capacities::for_counts(1, 4));
        assert!(
            captured
                .storage
                .observe(&fixture.memory(), 0, 0x100, Some(112), 1, true)
        );
        assert_eq!(captured.storage.events[0].snapshot.failure, None);
        assert_eq!(captured.storage.events[0].snapshot.omitted, 0);
        let rows = captured.convert().unwrap();
        assert_eq!(rows[0].actions.len(), 4);
        for (action, (_, name)) in rows[0].actions.iter().zip(forms) {
            assert!(action.name_key.is_none());
            assert_eq!(action.cached_name.as_deref(), Some(name.as_bytes()));
        }
    }

    #[test]
    fn cached_heap_name_past_the_prepared_arena_is_kept_whole_in_the_callback() {
        let _globals = crate::peace_settlements::test_globals();
        let mut fixture = crate::peace_settlement_tests::fixture::Fixture::new(1, 1);
        let name = vec![b'x'; 5_001];
        let heap = fixture.bytes.len();
        fixture.bytes.extend_from_slice(&name);
        fixture.bytes.push(0);
        let text = 0x20000 + engine_peace::peace_action::CACHED_NAME;
        fixture.u64(text, heap as u64);
        fixture.u64(text + engine_native_memory::msvc_string::LENGTH, name.len() as u64);
        fixture.u64(
            text + engine_native_memory::msvc_string::CAPACITY,
            name.len() as u64,
        );
        let demand = engine_peace::peace_settlement::demand(&fixture.memory(), 0, 0x100);
        assert_eq!(demand.failure, None);
        assert_eq!(demand.name_bytes, name.len());
        let capacity = Capacities::for_counts(1, 1);
        assert!(capacity.name_bytes < name.len());
        let mut captured = Captured::with_capacities(&capacity);
        assert!(
            captured
                .storage
                .observe(&fixture.memory(), 0, 0x100, Some(112), 1, true)
        );
        let snapshot = &captured.storage.events[0].snapshot;
        assert_eq!(snapshot.failure, None);
        assert_eq!(snapshot.omitted, 0);
        assert_eq!(snapshot.actions, 1);
        assert!(captured.storage.capacity().name_bytes >= name.len());
        let rows = captured.convert().unwrap();
        assert_eq!(
            rows[0].actions[0].cached_name.as_deref(),
            Some(name.as_slice())
        );
    }

    #[test]
    fn peace_unavailable_reason_signals_allocation_failure_once() {
        use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        let _globals = crate::peace_settlements::test_globals();
        static OWNER: AtomicU32 = AtomicU32::new(0);
        static FAILURES: AtomicUsize = AtomicUsize::new(0);
        fn owns_failure() -> bool {
            OWNER.load(Ordering::Acquire) == unsafe { GetCurrentThreadId() }
        }
        fn failed() {
            if owns_failure() {
                FAILURES.fetch_add(1, Ordering::Relaxed);
            }
        }
        fn stopped() -> bool {
            owns_failure() && FAILURES.load(Ordering::Relaxed) != 0
        }
        let mut captured = Captured::vacant();
        captured.fail("synthetic unavailable reason");
        FAILURES.store(0, Ordering::Relaxed);
        OWNER.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
        let tables = crate::test_support::with_allocations_denied(|| {
            engine_allocation::set_hooks(None, Some(failed));
            engine_allocation::set_stop_hook(Some(stopped));
            let tables = captured.tables();
            engine_allocation::set_stop_hook(None);
            engine_allocation::set_hooks(None, None);
            tables
        });
        OWNER.store(0, Ordering::Release);
        assert_eq!(FAILURES.load(Ordering::Relaxed), 1);
        assert!(matches!(&tables[0], TableView::Unavailable { reason } if reason.is_empty()));
    }

    #[test]
    fn peace_conversion_returns_allocation_failure() {
        let _globals = crate::peace_settlements::test_globals();
        let mut captured = Captured::with_capacities(&Capacities {
            events: 1,
            countries: 0,
            actions: 0,
            transfers: 0,
            name_bytes: 0,
        });
        captured.storage.used[0] = 1;
        let result = crate::test_support::with_allocations_denied(|| captured.convert());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::OutOfMemory);
    }
    #[test]
    fn peace_settlement_growth_captures_native_demand_past_former_caps() {
        use crate::peace_settlement_tests::fixture::Fixture;
        let count = 262_145;
        let mut captured = Captured::with_capacities(&Capacities {
            events: 1,
            countries: 0,
            actions: 1,
            transfers: 0,
            name_bytes: 0,
        });
        captured.storage.used[0] = 1;
        captured.storage.events[0].snapshot.omitted = count;
        captured.drained = Ok(0);
        assert!(
            matches!(captured.outcomes()[0], Outcome::Truncated { unread_rows, .. } if unread_rows == count as u32)
        );
        let mut capacity = captured.storage.capacity();
        captured.grow(&mut capacity);
        assert_eq!(capacity.events, count + 1);
        assert_eq!(capacity.countries, count);
        assert_eq!(capacity.actions, count + 1);
        assert_eq!(capacity.transfers, count);
        let mut fixture = Fixture::new(65_537, count);
        fixture.bytes[0x100 + engine_peace::peace_conference::FINALIZED] = 0;
        let roster = fixture.bytes.len();
        fixture.bytes.resize(roster + 8, 0);
        fixture.array(
            0x10000 + engine_native::cgame_state::PEACE_CONFERENCES_DATA,
            roster,
            1,
            8,
        );
        fixture.u64(roster, 0x100);
        captured.capture_bids(
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &fixture.bytes,
            },
            0,
            0x10000,
            91,
        );
        let [TableView::Complete(TableRows::PeaceSettlements(rows))] = captured.tables() else {
            panic!("native demand must be captured in full after growth");
        };
        assert_eq!(rows[0].winners.len(), 65_537);
        assert_eq!(rows[0].actions.len(), count);
        assert_eq!(rows[0].omitted, 0);
    }

    #[test]
    fn peace_settlement_bid_roster_exceeds_former_country_cap() {
        use crate::peace_settlement_tests::fixture::Fixture;
        let count = 65_537;
        let mut fixture = Fixture::new(0, 0);
        fixture.bytes[0x100 + engine_peace::peace_conference::FINALIZED] = 0;
        let roster = fixture.bytes.len();
        fixture.bytes.resize(roster + count * 8, 0);
        fixture.array(
            0x10000 + engine_native::cgame_state::PEACE_CONFERENCES_DATA,
            roster,
            count,
            8,
        );
        for index in 0..count {
            fixture.u64(roster + index * 8, 0x100);
        }
        let mut captured = Captured::with_capacities(&Capacities {
            events: count,
            countries: 0,
            actions: 0,
            transfers: 0,
            name_bytes: 0,
        });
        captured.capture_bids(
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &fixture.bytes,
            },
            0,
            0x10000,
            91,
        );
        assert!(matches!(captured.outcomes()[0], Outcome::Rows { len } if len == count));
        assert_eq!(captured.drained, Ok(0));
        assert_eq!(captured.storage.used[0], count);
    }
}
