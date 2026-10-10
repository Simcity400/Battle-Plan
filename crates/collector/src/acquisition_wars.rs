//! Owned war census within the callback's current country lease.

use crate::{acquisition_gather, sampling};
use engine::{Memory, country_memory::Cache, cwar_relation, walk::Outcome};
use std::sync::Arc;

pub(crate) struct Request {
    pub countries: Arc<Cache>,
    pub table: acquisition_gather::Request<Vec<cwar_relation::Row>>,
    pub snapshot: cwar_relation::OwnedSnapshotScratch,
    pub snapshot_sampled: bool,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Report {
    pub table: acquisition_gather::Report,
    pub table_ticks: i64,
}
impl Report {
    pub(crate) fn failed(reason: &'static str) -> Self {
        Self {
            table: acquisition_gather::Report {
                result: Outcome::Unavailable { reason },
            },
            table_ticks: 0,
        }
    }
}
impl Request {
    #[cfg(test)]
    pub(crate) fn run(&mut self, memory: &impl Memory) -> Report {
        self.run_with(memory, false)
    }

    pub(crate) fn run_growing(&mut self, memory: &impl Memory) -> Report {
        self.run_with(memory, true)
    }

    fn run_with(&mut self, memory: &impl Memory, growing: bool) -> Report {
        let _reads = crate::read_accounting::table(replay::rows::TableId::WarRelations);
        let Self {
            countries,
            table,
            snapshot,
            snapshot_sampled,
        } = self;
        let cached = countries.view(memory);
        let (base, game_state) = (table.base, table.game_state);
        let begin = sampling::qpc();
        *snapshot_sampled = true;
        let table = table.run(|rows| {
            if growing {
                cwar_relation::walk_owned_growing(memory, &cached, base, game_state, rows, snapshot)
            } else {
                cwar_relation::walk_owned(memory, &cached, base, game_state, rows, snapshot)
            }
        });
        Report {
            table,
            table_ticks: sampling::qpc() - begin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{acquisition::Pool, sampling::wars_test_kit as kit};
    use std::cell::RefCell;

    fn request(bytes: &[u8], capacity: usize) -> Request {
        let memory = kit::memory(bytes);
        let mut countries = Cache::try_with_capacity(1024).unwrap();
        assert_eq!(
            countries
                .prepare(&memory, kit::BASE, kit::GAME_STATE)
                .seed_status(),
            engine::country_memory::SeedStatus::Ready
        );
        let mut snapshot = cwar_relation::OwnedSnapshotScratch::try_with_capacity(5).unwrap();
        cwar_relation::walk_owned(
            &memory,
            &memory,
            kit::BASE,
            kit::GAME_STATE,
            &mut [cwar_relation::Row::default(); 4],
            &mut snapshot,
        );
        snapshot.grow(5).unwrap();
        Request {
            countries: Arc::new(countries),
            table: acquisition_gather::Request {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                payload: vec![
                    cwar_relation::Row {
                        cancel: 0xa5,
                        ..Default::default()
                    };
                    capacity
                ],
            },
            snapshot,
            snapshot_sampled: false,
        }
    }

    #[test]
    fn single_owned_wars_preserve_primary_rows_and_storage_at_every_capacity() {
        let bytes = kit::fixture();
        for capacity in [0, 1, 2, 4] {
            let memory = kit::memory(&bytes);
            let mut request = request(&bytes, capacity);
            let mut expected = request.table.payload.clone();
            let baseline = cwar_relation::walk(
                &request.countries.view(&memory),
                kit::BASE,
                kit::GAME_STATE,
                &mut expected,
            );
            let pointer = request.table.payload.as_ptr();
            let snapshot_capacity = request.snapshot.capacity();
            let report = request.run(&memory);
            assert_eq!(report.table.result, baseline);
            assert_eq!(request.table.payload, expected);
            assert_eq!(request.table.payload.as_ptr(), pointer);
            assert_eq!(request.snapshot.capacity(), snapshot_capacity);
            assert!(request.snapshot_sampled);
            if capacity >= 2 {
                assert_eq!(report.table.result, Outcome::Rows { len: 2 });
            }
        }
    }

    #[test]
    fn growing_owned_wars_fill_the_first_empty_row_buffer() {
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        assert_eq!(
            request(&bytes, 0).run(&memory).table.result,
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        let mut request = request(&bytes, 0);
        let expected = kit::direct(&memory, 2);
        let report = request.run_growing(&memory);
        assert_eq!(report.table.result, Outcome::Rows { len: 2 });
        assert_eq!(report.table.result, expected.owned_outcome);
        assert_eq!(&request.table.payload[..2], &expected.owned);
        assert_eq!(request.table.payload[1].first_country, 3);
        assert_eq!(request.table.payload[1].second_country, 1);
        assert!(request.snapshot_sampled);
    }

    struct Traced<'a> {
        bytes: &'a [u8],
        calls: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, output.len()));
            if self.fault == Some(address) {
                return false;
            }
            kit::memory(self.bytes).copy(address, output)
        }
    }

    #[test]
    fn wars_first_row_fault_is_returned_without_second_observation() {
        let bytes = kit::fixture();
        let memory = Traced {
            bytes: &bytes,
            calls: RefCell::default(),
            fault: Some(kit::WAR1 + cwar_relation::FIRST_CASUALTIES),
        };
        let mut request = request(&bytes, 4);
        let report = request.run(&memory);
        assert!(matches!(report.table.result, Outcome::Unavailable { .. }));
        assert_eq!(
            memory
                .calls
                .borrow()
                .iter()
                .filter(|(at, _)| *at == kit::WAR1 + cwar_relation::FIRST_CASUALTIES)
                .count(),
            1
        );
    }

    #[test]
    fn wars_payload_and_snapshot_return_through_caller_and_worker_execution() {
        for workers in [0, 2] {
            let bytes = kit::fixture();
            let owned = request(&bytes, 4);
            let pointer = owned.table.payload.as_ptr();
            let capacity = owned.snapshot.capacity();
            let pool = Pool::start(workers, 1, move |request: &mut Request| {
                request.run(&kit::memory(&bytes))
            })
            .unwrap();
            let mut pending = Some(owned);
            // SAFETY: synthetic source and owned outputs survive until collection.
            let ticket = unsafe { pool.submit(&mut pending) }.unwrap();
            let completion = pool.finish(ticket).unwrap();
            assert_eq!(
                completion.report.unwrap().table.result,
                Outcome::Rows { len: 2 }
            );
            assert_eq!(completion.request.table.payload.as_ptr(), pointer);
            assert_eq!(completion.request.snapshot.capacity(), capacity);
            assert!(completion.request.snapshot_sampled);
            assert_eq!(pool.calibration().uncollected, 0);
        }
    }
}
