
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x2a4ba28);
pub const PERSISTENT_SUBOBJECT: usize = 0x18;
pub const PERSISTENT_VTABLE_RVA: Rva = Rva(0x2a4ba60);
#[cfg(test)]
pub const GROUPS_DATA: usize = 0x20;
pub const GROUPS_COUNT: usize = 0x2c;

use engine_native_memory::{Memory, cpdx_array::Array, read, walk::{Buffer, Outcome}};
use crate::{army_upgrade_request_row, carmy_upgrades_requests, cupgrade_delivery};
use engine_native_parts::equipment_entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcomes {
    pub groups: Outcome,
    pub requests: Outcome,
    pub deliveries: Outcome,
    pub produced: Outcome,
}

impl Outcomes {
    pub fn unavailable(reason: &'static str) -> Self {
        Self {
            groups: Outcome::Unavailable { reason },
            requests: Outcome::Unavailable { reason },
            deliveries: Outcome::Unavailable { reason },
            produced: Outcome::Unavailable { reason },
        }
    }
}

pub fn count(memory: &impl Memory, base: usize, sources: &[engine_native::cequipment_variant::pool::DivisionSource]) -> Outcomes {
    walk_buffer(memory, base, sources, Buffer::Count, Buffer::Count, Buffer::Count, Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    sources: &[engine_native::cequipment_variant::pool::DivisionSource],
    groups: &mut Vec<carmy_upgrades_requests::Row>,
    requests: &mut Vec<army_upgrade_request_row::Row>,
    deliveries: &mut Vec<cupgrade_delivery::Row>,
    produced: &mut Vec<cupgrade_delivery::ProducedRow>,
) -> Outcomes {
    walk_buffer(
        memory,
        base,
        sources,
        Buffer::Growing(groups),
        Buffer::Growing(requests),
        Buffer::Growing(deliveries),
        Buffer::Growing(produced),
    )
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    sources: &[engine_native::cequipment_variant::pool::DivisionSource],
    groups: Buffer<'_, carmy_upgrades_requests::Row>,
    requests: Buffer<'_, army_upgrade_request_row::Row>,
    deliveries: Buffer<'_, cupgrade_delivery::Row>,
    produced: Buffer<'_, cupgrade_delivery::ProducedRow>,
) -> Outcomes {
    let mut groups = Sink::new(groups);
    let mut requests = Sink::new(requests);
    let mut deliveries = Sink::new(deliveries);
    let mut produced = Sink::new(produced);
    let result = (|| {
        for source in sources {
            let outer = source
                .pointer
                .checked_add(engine_native::carmy::REQUESTS)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|&pointer| pointer != 0)
                .ok_or("army requests pointer unreadable or null")?;
            let group_pointer = outer
                .checked_add(carmy_upgrades_requests::IN_ARMY_REQUESTS)
                .ok_or("army upgrade group address overflow")?;
            let group = carmy_upgrades_requests::row(memory, base, group_pointer, source)?;
            groups.push(|| Ok(group));
            if !requests.failed() {
                match vector(
                    memory,
                    group_pointer,
                    carmy_upgrades_requests::REQUESTS_DATA,
                    army_upgrade_request_row::STRIDE,
                    "army upgrade request vector unreadable or invalid",
                ) {
                    Ok(rows) => {
                        requests.rows(rows, army_upgrade_request_row::STRIDE, |address, index| {
                            army_upgrade_request_row::row(
                                memory,
                                base,
                                address,
                                source.country,
                                group.army,
                                index,
                            )
                        });
                    }
                    Err(reason) => requests.fail(reason),
                }
            }
            if deliveries.failed() && produced.failed() {
                continue;
            }
            let child_deliveries = match vector(
                memory,
                group_pointer,
                carmy_upgrades_requests::DELIVERIES_DATA,
                carmy_upgrades_requests::DELIVERY_ENTRY_STRIDE,
                "army upgrade delivery vector unreadable or invalid",
            ) {
                Ok(rows) => {
                    deliveries.prepare(rows.count as usize);
                    rows
                }
                Err(reason) => {
                    deliveries.fail(reason);
                    produced.fail(reason);
                    continue;
                }
            };
            for delivery_ordinal in 0..child_deliveries.count {
                if deliveries.failed() && produced.failed() {
                    break;
                }
                let header = (|| {
                    let address = child_deliveries
                        .element(
                            delivery_ordinal,
                            carmy_upgrades_requests::DELIVERY_ENTRY_STRIDE,
                        )
                        .ok_or("army upgrade delivery slot overflow")?;
                    let pointer = carmy_upgrades_requests::pointer(memory, address)?;
                    cupgrade_delivery::authenticate(memory, base, pointer)
                })();
                let header = match header {
                    Ok(header) => header,
                    Err(reason) => {
                        deliveries.fail(reason);
                        produced.fail(reason);
                        break;
                    }
                };
                deliveries.push(|| {
                    cupgrade_delivery::row(
                        memory,
                        &header,
                        source.country,
                        group.army,
                        delivery_ordinal,
                    )
                });
                if !produced.failed() {
                    match cupgrade_delivery::produced(memory, &header) {
                        Ok(rows) => {
                            produced.rows(rows, equipment_entry::STRIDE, |address, index| {
                                cupgrade_delivery::produced_row(
                                    memory,
                                    base,
                                    address,
                                    (source.country, group.army, delivery_ordinal),
                                    index,
                                )
                            })
                        }
                        Err(reason) => produced.fail(reason),
                    }
                }
            }
        }
        Ok::<_, &'static str>(())
    })();
    match result {
        Err(reason) => Outcomes::unavailable(reason),
        Ok(()) => Outcomes {
            groups: groups.finish(),
            requests: requests.finish(),
            deliveries: deliveries.finish(),
            produced: produced.finish(),
        },
    }
}

fn vector(
    memory: &impl Memory,
    address: usize,
    offset: usize,
    stride: usize,
    reason: &'static str,
) -> Result<Array, &'static str> {
    address
        .checked_add(offset)
        .and_then(|p| Array::read(memory, p, stride))
        .ok_or(reason)
}

struct Sink<'a, R> {
    rows: Buffer<'a, R>,
    len: usize,
    unread_rows: u32,
    reason: Option<&'static str>,
}

impl<'a, R: Default> Sink<'a, R> {
    fn new(rows: Buffer<'a, R>) -> Self {
        Self {
            rows,
            len: 0,
            unread_rows: 0,
            reason: None,
        }
    }

    fn failed(&self) -> bool {
        self.reason.is_some()
    }

    fn fail(&mut self, reason: &'static str) {
        self.reason.get_or_insert(reason);
    }

    fn omit(&mut self, count: u32) {
        match self.unread_rows.checked_add(count) {
            Some(total) => self.unread_rows = total,
            None => self.fail("army upgrade unread row count overflow"),
        }
    }

    fn prepare(&mut self, count: usize) {
        if self.failed() {
            return;
        }
        let result = self
            .len
            .checked_add(count)
            .ok_or("army upgrade sizing overflow")
            .and_then(|required| self.rows.ensure(required));
        if let Err(reason) = result {
            self.fail(reason);
        }
    }

    fn push(&mut self, read_row: impl FnOnce() -> Result<R, &'static str>) {
        self.prepare(1);
        if self.failed() {
            return;
        }
        if self.len == self.rows.len() {
            self.omit(1);
            return;
        }
        match read_row() {
            Ok(row) => {
                self.rows[self.len] = row;
                self.len += 1;
            }
            Err(reason) => self.fail(reason),
        }
    }

    fn rows(
        &mut self,
        array: Array,
        stride: usize,
        mut read_row: impl FnMut(usize, u32) -> Result<R, &'static str>,
    ) {
        self.prepare(array.count as usize);
        if self.failed() {
            return;
        }
        let take = (self.rows.len() - self.len).min(array.count as usize) as u32;
        for index in 0..take {
            let Some(address) = array.element(index, stride) else {
                self.fail("army upgrade leaf row address overflow");
                return;
            };
            self.push(|| read_row(address, index));
            if self.failed() {
                return;
            }
        }
        self.omit(array.count - take);
    }

    fn finish(self) -> Outcome {
        match (self.reason, self.unread_rows) {
            (Some(reason), _) => Outcome::Unavailable { reason },
            (None, 0) => Outcome::Rows { len: self.len },
            (None, unread_rows) => Outcome::Truncated {
                len: self.len,
                unread_rows,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::{
        carmy,
        cequipment_variant::{self, pool as cequipment_variant_pool},
    };
    use engine_native_memory::memory::SliceMemory;

    const STATUS: usize = 0x6000;
    const GROUPS: usize = 0x6800;
    const GROUP: usize = 0x7000;
    const ARMY: usize = 0x8000;
    const REQUESTS: usize = 0xa000;
    const DELIVERIES: usize = 0xa100;
    const DELIVERY: usize = 0xb000;
    const EMPTY_DELIVERY: usize = 0xb100;
    const PRODUCED: usize = 0xc000;
    const EQUIPMENT: usize = 0xd000;

    fn put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn array(bytes: &mut [u8], at: usize, pointer: usize, count: u32) {
        put64(bytes, at, pointer as u64);
        put32(bytes, at + 8, count);
        put32(bytes, at + 12, count);
    }
    fn sources() -> [cequipment_variant_pool::DivisionSource; 1] {
        [cequipment_variant_pool::DivisionSource {
            pointer: ARMY,
            division: 9000,
            template: 0,
            country: 3,
        }]
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0xe000];
        put64(&mut bytes, STATUS, VTABLE_RVA.0 as u64);
        put64(
            &mut bytes,
            STATUS + PERSISTENT_SUBOBJECT,
            PERSISTENT_VTABLE_RVA.0 as u64,
        );
        array(&mut bytes, STATUS + GROUPS_DATA, GROUPS, 1);
        put64(&mut bytes, GROUPS, GROUP as u64);
        put64(
            &mut bytes,
            GROUP,
            carmy_upgrades_requests::VTABLE_RVA.0 as u64,
        );
        put32(
            &mut bytes,
            GROUP + carmy_upgrades_requests::PRIORITY,
            u32::MAX,
        );
        put64(
            &mut bytes,
            GROUP + carmy_upgrades_requests::PERSISTENT_SUBOBJECT,
            carmy_upgrades_requests::PERSISTENT_VTABLE_RVA.0 as u64,
        );
        put64(
            &mut bytes,
            GROUP + carmy_upgrades_requests::ARMY,
            ARMY as u64,
        );
        put64(
            &mut bytes,
            GROUP + carmy_upgrades_requests::STATUS,
            STATUS as u64,
        );
        array(
            &mut bytes,
            GROUP + carmy_upgrades_requests::REQUESTS_DATA,
            REQUESTS,
            2,
        );
        array(
            &mut bytes,
            GROUP + carmy_upgrades_requests::DELIVERIES_DATA,
            DELIVERIES,
            2,
        );
        put64(&mut bytes, ARMY, carmy::VTABLE_RVA.0 as u64);
        put64(
            &mut bytes,
            ARMY + carmy::REQUESTS,
            (GROUP - carmy_upgrades_requests::IN_ARMY_REQUESTS) as u64,
        );
        put64(
            &mut bytes,
            ARMY + carmy::SECONDARY_BASE,
            carmy::SECONDARY_VTABLE_RVA.0 as u64,
        );
        put64(&mut bytes, ARMY + engine_native::cunit::HANDLE, 9000);
        put64(
            &mut bytes,
            EQUIPMENT,
            engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64,
        );
        put64(&mut bytes, EQUIPMENT + cequipment_variant::TYPE, 70);
        for ordinal in 0..2 {
            let request = REQUESTS + ordinal * army_upgrade_request_row::STRIDE;
            put64(&mut bytes, request, EQUIPMENT as u64);
            put64(
                &mut bytes,
                request + 8,
                (i64::MIN + 7 + ordinal as i64) as u64,
            );
            put64(&mut bytes, request + 16, (i64::MAX - 9) as u64);
            let produced = PRODUCED + ordinal * equipment_entry::STRIDE;
            put64(&mut bytes, produced, EQUIPMENT as u64);
            put64(&mut bytes, produced + 8, (i64::MIN + ordinal as i64) as u64);
        }
        for (ordinal, pointer, status) in [(0, DELIVERY, 1), (1, EMPTY_DELIVERY, 2)] {
            put64(&mut bytes, DELIVERIES + ordinal * 8, pointer as u64);
            put64(&mut bytes, pointer, cupgrade_delivery::VTABLE_RVA.0 as u64);
            put32(&mut bytes, pointer + cupgrade_delivery::STATUS, status);
            put64(
                &mut bytes,
                pointer + cupgrade_delivery::PROGRESS,
                i64::MIN as u64,
            );
            put64(
                &mut bytes,
                pointer + cupgrade_delivery::TOTAL_PROGRESS,
                i64::MAX as u64,
            );
            put64(
                &mut bytes,
                pointer + cupgrade_delivery::PRODUCED_EQUIPMENT_POOL,
                cequipment_variant_pool::VTABLE_RVA.0 as u64,
            );
            bytes[pointer
                + cupgrade_delivery::PRODUCED_EQUIPMENT_POOL
                + cequipment_variant_pool::ALLOW_ZERO] = 0xff;
        }
        array(
            &mut bytes,
            DELIVERY + cupgrade_delivery::PRODUCED_EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
            PRODUCED,
            2,
        );
        bytes
    }

    #[test]
    fn preserves_hierarchy_order_raw_precision_and_empty_deliveries() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut groups = vec![carmy_upgrades_requests::Row::default(); 4];
        let mut requests = vec![army_upgrade_request_row::Row::default(); 4];
        let mut deliveries = vec![cupgrade_delivery::Row::default(); 4];
        let mut produced = vec![cupgrade_delivery::ProducedRow::default(); 4];
        assert_eq!(
            walk(
                &memory,
                0,
                &sources(),
                &mut groups,
                &mut requests,
                &mut deliveries,
                &mut produced
            ),
            Outcomes {
                groups: Outcome::Rows { len: 1 },
                requests: Outcome::Rows { len: 2 },
                deliveries: Outcome::Rows { len: 2 },
                produced: Outcome::Rows { len: 2 }
            }
        );
        assert_eq!(
            groups[0],
            carmy_upgrades_requests::Row {
                country: 3,
                army: 9000,
                priority: u32::MAX
            }
        );
        assert_eq!(
            requests[0],
            army_upgrade_request_row::Row {
                country: 3,
                army: 9000,
                ordinal: 0,
                equipment: 70,
                request_raw: i64::MIN + 7,
                total_raw: i64::MAX - 9,
            }
        );
        assert_eq!(requests[1].ordinal, 1);
        assert_eq!(requests[1].request_raw, i64::MIN + 8);
        assert_eq!(deliveries[0].progress_raw, Some(i64::MIN));
        assert_eq!(deliveries[0].total_progress_raw, i64::MAX);
        assert_eq!(deliveries[0].allow_zero_entries, 0xff);
        assert_eq!(deliveries[1].progress_raw, None);
        assert_eq!(deliveries[1].ordinal, 1);
        assert_eq!(
            produced[1],
            cupgrade_delivery::ProducedRow {
                country: 3,
                army: 9000,
                delivery_ordinal: 0,
                ordinal: 1,
                equipment: 70,
                amount_raw: i64::MIN + 1,
            }
        );
    }

    struct FaultMemory<'a> {
        bytes: &'a [u8],
        first: usize,
        end: usize,
    }
    impl Memory for FaultMemory<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address < self.end && address.saturating_add(destination.len()) > self.first {
                return false;
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }

    #[test]
    fn nonprogress_status_never_reads_inactive_progress_and_field_faults_are_scoped() {
        let bytes = fixture();
        let mut groups = vec![carmy_upgrades_requests::Row::default(); 1];
        let mut requests = vec![army_upgrade_request_row::Row::default(); 2];
        let mut deliveries = vec![cupgrade_delivery::Row::default(); 2];
        let mut produced = vec![cupgrade_delivery::ProducedRow::default(); 2];
        for (first, width, expected) in [
            (
                EMPTY_DELIVERY + cupgrade_delivery::PROGRESS,
                8,
                Outcome::Rows { len: 2 },
            ),
            (
                DELIVERY + cupgrade_delivery::PROGRESS,
                8,
                Outcome::Unavailable {
                    reason: "army upgrade delivery progress unreadable",
                },
            ),
            (
                DELIVERY + cupgrade_delivery::STATUS,
                4,
                Outcome::Unavailable {
                    reason: "army upgrade delivery status unreadable",
                },
            ),
        ] {
            let memory = FaultMemory {
                bytes: &bytes,
                first,
                end: first + width,
            };
            let result = walk(
                &memory,
                0,
                &sources(),
                &mut groups,
                &mut requests,
                &mut deliveries,
                &mut produced,
            );
            assert_eq!(result.deliveries, expected);
            assert_eq!(result.groups, Outcome::Rows { len: 1 });
            assert_eq!(result.requests, Outcome::Rows { len: 2 });
            assert_eq!(result.produced, Outcome::Rows { len: 2 });
        }
    }

    #[test]
    fn an_unreadable_request_fails_its_table() {
        let bytes = fixture();
        let memory = FaultMemory {
            bytes: &bytes,
            first: REQUESTS + army_upgrade_request_row::STRIDE,
            end: REQUESTS + 2 * army_upgrade_request_row::STRIDE,
        };
        let mut groups = vec![];
        let mut deliveries = vec![cupgrade_delivery::Row::default(); 1];
        let mut produced = vec![cupgrade_delivery::ProducedRow::default(); 1];
        let mut requests = vec![army_upgrade_request_row::Row::default(); 1];
        assert_eq!(
            walk(
                &memory,
                0,
                &sources(),
                &mut groups,
                &mut requests,
                &mut deliveries,
                &mut produced
            )
            .requests,
            Outcome::Unavailable {
                reason: "army upgrade request unreadable"
            }
        );
    }

    #[test]
    fn bad_child_vector_is_scoped_but_invalid_parent_never_becomes_empty() {
        let mut bytes = fixture();
        let mut groups = vec![carmy_upgrades_requests::Row::default(); 1];
        let mut requests = vec![army_upgrade_request_row::Row::default(); 2];
        let mut deliveries = vec![cupgrade_delivery::Row::default(); 2];
        let mut produced = vec![cupgrade_delivery::ProducedRow::default(); 2];
        put32(
            &mut bytes,
            GROUP + carmy_upgrades_requests::REQUESTS_CAPACITY,
            u32::MAX,
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let result = walk(
            &memory,
            0,
            &sources(),
            &mut groups,
            &mut requests,
            &mut deliveries,
            &mut produced,
        );
        assert_eq!(
            result.requests,
            Outcome::Unavailable {
                reason: "army upgrade request vector unreadable or invalid"
            }
        );
        assert_eq!(result.groups, Outcome::Rows { len: 1 });
        assert_eq!(result.deliveries, Outcome::Rows { len: 2 });
        assert_eq!(result.produced, Outcome::Rows { len: 2 });
        put64(&mut bytes, GROUP + carmy_upgrades_requests::ARMY, 0);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk(
                &memory,
                0,
                &sources(),
                &mut groups,
                &mut requests,
                &mut deliveries,
                &mut produced
            ),
            Outcomes::unavailable("army upgrade group army back-reference mismatch")
        );
    }

    #[test]
    fn delivery_only_group_remains_after_unregistering_and_empty_owned_group_is_retained() {
        let mut bytes = fixture();
        array(&mut bytes, STATUS + GROUPS_DATA, 0, 0);
        array(
            &mut bytes,
            GROUP + carmy_upgrades_requests::REQUESTS_DATA,
            0,
            0,
        );
        let mut groups = vec![carmy_upgrades_requests::Row::default(); 1];
        let mut requests = vec![];
        let mut deliveries = vec![cupgrade_delivery::Row::default(); 2];
        let mut produced = vec![cupgrade_delivery::ProducedRow::default(); 2];
        let memory = FaultMemory {
            bytes: &bytes,
            first: STATUS,
            end: STATUS + 0x40,
        };
        assert_eq!(
            walk(
                &memory,
                0,
                &sources(),
                &mut groups,
                &mut requests,
                &mut deliveries,
                &mut produced
            ),
            Outcomes {
                groups: Outcome::Rows { len: 1 },
                requests: Outcome::Rows { len: 0 },
                deliveries: Outcome::Rows { len: 2 },
                produced: Outcome::Rows { len: 2 }
            }
        );
        assert_eq!(
            (groups[0].army, groups[0].country, groups[0].priority),
            (9000, 3, u32::MAX)
        );
        assert_eq!(
            (
                produced[1].army,
                produced[1].ordinal,
                produced[1].amount_raw
            ),
            (9000, 1, i64::MIN + 1)
        );
        array(
            &mut bytes,
            GROUP + carmy_upgrades_requests::DELIVERIES_DATA,
            0,
            0,
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk(
                &memory,
                0,
                &sources(),
                &mut groups,
                &mut requests,
                &mut deliveries,
                &mut produced
            ),
            Outcomes {
                groups: Outcome::Rows { len: 1 },
                requests: Outcome::Rows { len: 0 },
                deliveries: Outcome::Rows { len: 0 },
                produced: Outcome::Rows { len: 0 }
            }
        );
        assert_eq!(
            walk(
                &memory,
                0,
                &[],
                &mut groups,
                &mut requests,
                &mut deliveries,
                &mut produced
            ),
            Outcomes {
                groups: Outcome::Rows { len: 0 },
                requests: Outcome::Rows { len: 0 },
                deliveries: Outcome::Rows { len: 0 },
                produced: Outcome::Rows { len: 0 }
            }
        );
    }

    #[test]
    fn unreadable_or_null_owning_requests_pointer_is_not_an_empty_group() {
        let mut bytes = fixture();
        let mut groups = vec![carmy_upgrades_requests::Row::default(); 1];
        let mut requests = vec![];
        let mut deliveries = vec![];
        let mut produced = vec![];
        let memory = FaultMemory {
            bytes: &bytes,
            first: ARMY + carmy::REQUESTS,
            end: ARMY + carmy::REQUESTS + 8,
        };
        assert_eq!(
            walk(
                &memory,
                0,
                &sources(),
                &mut groups,
                &mut requests,
                &mut deliveries,
                &mut produced
            ),
            Outcomes::unavailable("army requests pointer unreadable or null")
        );
        put64(&mut bytes, ARMY + carmy::REQUESTS, 0);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk(
                &memory,
                0,
                &sources(),
                &mut groups,
                &mut requests,
                &mut deliveries,
                &mut produced
            ),
            Outcomes::unavailable("army requests pointer unreadable or null")
        );
    }
}
