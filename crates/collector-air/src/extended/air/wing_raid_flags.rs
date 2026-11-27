use engine_allocation as allocation;
use engine_mutations::air_wing_equipment_mutations as sites;
use engine_native_memory::Memory;
use engine_native_air::{cair_wing, craid_instance};

fn demanded(row: &cair_wing::Row) -> bool {
    row.air_base != 0
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    pointer: usize,
    wing: u64,
    certificate: Option<(u64, u64)>,
    slot: usize,
    raw: Option<u64>,
    lifetime: Option<(usize, (u64, u64), usize)>,
    value: Option<bool>,
}
#[derive(Default, Debug)]
pub struct Store {
    previous: Vec<Entry>,
    current: Vec<Entry>,
    cursor: usize,
}
impl Store {
    pub fn reference(
        &self,
        pointer: usize,
        wing: u64,
    ) -> Option<cair_wing::transport_operands::RaidReference> {
        let entry = self
            .current
            .last()
            .filter(|entry| entry.pointer == pointer && entry.wing == wing)?;
        Some(cair_wing::transport_operands::RaidReference {
            raw: entry.raw?,
            resolves: entry.value?,
        })
    }
    pub fn begin(&mut self, wings: usize, invalidate: bool) {
        if invalidate {
            self.current.clear();
        }
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.clear();
        self.cursor = 0;
        self.ensure_capacity(wings);
    }
    pub fn ensure_capacity(&mut self, wings: usize) -> bool {
        if wings <= self.current.capacity() {
            return true;
        }
        let additional = wings.saturating_sub(self.current.len());
        allocation::advisory(|| allocation::reserve(&mut self.current, additional)).is_ok()
    }
    pub fn finish(&mut self) {
        self.current.sort_unstable_by_key(|row| row.pointer);
        let mut at = 0;
        while at < self.current.len() {
            let mut end = at + 1;
            while end < self.current.len() && self.current[end].pointer == self.current[at].pointer
            {
                end += 1;
            }
            if end > at + 1 {
                self.current.drain(at..end);
            } else {
                at = end;
            }
        }
        self.previous.clear();
    }
    pub fn allocated_bytes(&self) -> u64 {
        ((self.previous.capacity() + self.current.capacity()) * size_of::<Entry>()) as u64
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        row: &cair_wing::Row,
        observed_pointer: Option<usize>,
        observed_token: Option<(u64, u64)>,
    ) -> Option<bool> {
        if !demanded(row) {
            return None;
        }
        self.capture_observed(
            memory,
            base,
            pointer,
            row,
            observed_pointer,
            observed_token,
            collector_observers::wing_changes::map_sequence_at,
            collector_observers::wing_changes::map_quiet,
        )
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "observer callbacks separate live capture from unit-test registry injection"
    )]
    fn capture_observed(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        row: &cair_wing::Row,
        observed_pointer: Option<usize>,
        observed_token: Option<(u64, u64)>,
        sequence: impl FnMut(usize, usize) -> Option<((u64, u64), usize)>,
        quiet: impl Fn() -> Option<(u64, u64)>,
    ) -> Option<bool> {
        self.capture_with_at(
            pointer,
            row.id,
            sequence,
            &quiet,
            || {
                if let Some(observed) = row.raid_reference
                    && observed_token.is_some_and(|token| Some(token) == quiet())
                {
                    return Some(observed.raw);
                }
                pointer
                    .checked_add(sites::RAID_REFERENCE)
                    .and_then(|at| engine_native_memory::read(memory, at))
            },
            |raw| {
                if let Some(observed) = row.raid_reference.filter(|value| value.raw == raw)
                    && let Some(pointer) = observed_pointer
                    && observed.resolves == (pointer != 0)
                    && observed_token.is_some_and(|token| Some(token) == quiet())
                {
                    return Some((pointer != 0).then_some(pointer));
                }
                resolve(memory, base, raw).ok()
            },
        )
    }
    #[cfg(test)]
    fn capture_with(
        &mut self,
        pointer: usize,
        wing: u64,
        mut sequence: impl FnMut(usize) -> Option<(u64, u64)>,
        quiet: impl FnMut() -> Option<(u64, u64)>,
        read: impl FnOnce() -> Option<u64>,
        resolve: impl FnOnce(u64) -> Option<Option<usize>>,
    ) -> Option<bool> {
        self.capture_with_at(
            pointer,
            wing,
            |at, _| sequence(at).map(|c| (c, usize::MAX)),
            quiet,
            read,
            resolve,
        )
    }
    fn capture_with_at(
        &mut self,
        pointer: usize,
        wing: u64,
        mut sequence: impl FnMut(usize, usize) -> Option<((u64, u64), usize)>,
        mut quiet: impl FnMut() -> Option<(u64, u64)>,
        read: impl FnOnce() -> Option<u64>,
        resolve: impl FnOnce(u64) -> Option<Option<usize>>,
    ) -> Option<bool> {
        if self.current.len() == self.current.capacity() {
            return None;
        }
        let receiver = pointer.checked_add(sites::RAID_REFERENCE)?;
        let found = if let Some(entry) = self.previous.get(self.cursor)
            && entry.pointer == pointer
        {
            self.cursor += 1;
            Some(*entry)
        } else {
            self.previous
                .binary_search_by_key(&pointer, |row| row.pointer)
                .ok()
                .map(|at| {
                    self.cursor = at + 1;
                    self.previous[at]
                })
        };
        let held_slot = found.map_or(usize::MAX, |row| row.slot);
        let (certificate, slot) = match sequence(receiver, held_slot) {
            Some((certificate, slot)) => (Some(certificate), slot),
            None => (None, usize::MAX),
        };
        let held = certificate.and_then(|certificate| {
            found.filter(|row| row.wing == wing && row.certificate == Some(certificate))
        });
        if let Some(mut held) = held {
            let same = match held.lifetime {
                None => true,
                Some((key, expected, key_slot)) => {
                    let (now, next_slot) = sequence(key, key_slot)?;
                    held.lifetime = Some((key, expected, next_slot));
                    now == expected && sequence(receiver, slot).map(|(c, _)| c) == certificate
                }
            };
            if same {
                held.slot = slot;
                self.current.push(held);
                return held.value;
            }
        }
        let token = quiet()?;
        let raw = match held {
            Some(held) => held.raw,
            None => read(),
        };
        let (value, lifetime) = match raw {
            None => (None, None),
            Some(0) => (Some(false), None),
            Some(raw) => match resolve(raw) {
                None => (None, None),
                Some(None) => (Some(false), None),
                Some(Some(_)) if certificate.is_none() => (Some(true), None),
                Some(Some(pointer)) => match pointer
                    .checked_add(sites::RAID_LIFETIME_KEY)
                    .and_then(|key| {
                        sequence(key, usize::MAX)
                            .map(|(certificate, key_slot)| (key, certificate, key_slot))
                    }) {
                    None => (None, None),
                    Some(lifetime) => (Some(true), Some(lifetime)),
                },
            },
        };
        if sequence(receiver, slot).map(|(c, _)| c) != certificate
            || quiet() != Some(token)
            || lifetime
                .is_some_and(|(key, held, key_slot)| sequence(key, key_slot).map(|(c, _)| c) != Some(held))
        {
            return None;
        }
        self.current.push(Entry {
            pointer,
            wing,
            certificate,
            slot,
            raw,
            lifetime,
            value,
        });
        value
    }
}

fn resolve(memory: &impl Memory, base: usize, raw: u64) -> Result<Option<usize>, &'static str> {
    if raw as u32 != craid_instance::REFERENCE_TYPE_VALUE {
        return Err("wing raid reference type unsupported");
    }
    let Some(pointer) = engine_native_parts::creference_registry::resolve(memory, base, raw)? else {
        return Ok(None);
    };
    let identity =
        engine_native_memory::read::<[u8; 16]>(memory, pointer).ok_or("wing raid identity unreadable")?;
    let expected = engine_sites::at(base, craid_instance::VTABLE_RVA)
        .ok_or("wing raid module address overflow")? as u64;
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected
        || u64::from_le_bytes(identity[8..].try_into().unwrap()) != raw
    {
        return Err("wing raid full identity mismatch");
    }
    Ok(Some(pointer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_transfer_demands_raid_identity_independently_of_mission_admission() {
        let mut row = cair_wing::Row {
            air_base: 17,
            eligibility_veto: 1,
            eligibility_counter: 2,
            eligibility_limit: 1,
            ..Default::default()
        };
        assert!(demanded(&row));
        row.mission_mask = 1;
        assert!(demanded(&row));
        row.eligibility_veto = 0;
        row.eligibility_counter = 1;
        assert!(demanded(&row));
        row.mission_region = Some(7);
        assert!(demanded(&row));
        row.mission_region = None;
        row.mission_mask = 0x800;
        assert!(demanded(&row));
        row.air_base = 0;
        assert!(!demanded(&row));
    }
    #[test]
    fn quiet_wings_read_neither_raw_references_nor_the_registry() {
        let mut store = Store::default();
        store.begin(0, true);
        for wing in 1..=32 {
            assert!(store.ensure_capacity(wing as usize));
            assert_eq!(
                store.capture_with(
                    wing as usize * 4096,
                    wing,
                    |_| Some((1, 0)),
                    || Some((1, 0)),
                    || Some(wing),
                    |_| Some(Some(0x100_00000 + wing as usize * 4096))
                ),
                Some(true)
            );
            assert_eq!(
                store.reference(wing as usize * 4096, wing),
                Some(cair_wing::transport_operands::RaidReference {
                    raw: wing,
                    resolves: true
                })
            );
        }
        store.finish();
        store.begin(32, false);
        for wing in 1..=32 {
            assert_eq!(
                store.capture_with(
                    wing as usize * 4096,
                    wing,
                    |_| Some((1, 0)),
                    || panic!("quiet carry needs no global token"),
                    || panic!("quiet raw read"),
                    |_| panic!("quiet registry probe")
                ),
                Some(true)
            );
            assert_eq!(
                store.reference(wing as usize * 4096, wing),
                Some(cair_wing::transport_operands::RaidReference {
                    raw: wing,
                    resolves: true
                })
            );
        }
    }
    #[test]
    fn a_retired_raid_reprobes_only_liveness_and_carries_a_stale_full_reference_as_false() {
        let mut store = Store::default();
        store.begin(1, true);
        assert_eq!(
            store.capture_with(
                0x1000,
                1,
                |_| Some((1, 0)),
                || Some((1, 0)),
                || Some(77),
                |_| Some(Some(0x9000))
            ),
            Some(true)
        );
        store.finish();
        store.begin(1, false);
        assert_eq!(
            store.capture_with(
                0x1000,
                1,
                |key| Some((1, u64::from(key == 0x9001))),
                || Some((1, 1)),
                || panic!("unchanged wing raw reference"),
                |raw| {
                    assert_eq!(raw, 77);
                    Some(None)
                }
            ),
            Some(false)
        );
        store.finish();
        store.begin(1, false);
        assert_eq!(
            store.capture_with(
                0x1000,
                1,
                |_| Some((1, 0)),
                || Some((1, 1)),
                || panic!("raw polling"),
                |_| panic!("stale reference polling")
            ),
            Some(false)
        );
    }
    #[test]
    fn absent_certificates_storage_and_overlapping_lifetimes_never_guess_false() {
        let mut store = Store::default();
        assert_eq!(
            store.capture_with(
                0x1000,
                1,
                |_| panic!(),
                || panic!(),
                || panic!(),
                |_| panic!()
            ),
            None
        );
        store.begin(1, true);
        assert_eq!(
            store.capture_with(0x1000, 1, |_| None, || None, || panic!(), |_| panic!()),
            None,
            "unobserved"
        );
        let mut revisions = 0;
        assert_eq!(
            store.capture_with(
                0x1000,
                1,
                |_| Some((1, 0)),
                || {
                    revisions += 1;
                    Some((1, revisions))
                },
                || Some(77),
                |_| Some(Some(0x9000))
            ),
            None
        );
        assert!(store.current.is_empty());
    }
    #[test]
    fn an_observed_registry_without_a_table_reads_references_between_quiet_tokens() {
        let registry = collector_certificates::watch_registry::WatchRegistry::new();
        registry.set_enabled(true);
        let mut store = Store::default();
        store.begin(3, true);
        assert_eq!(
            store.capture_with(
                0x1000,
                1,
                |at| registry.sequence(at),
                || registry.quiet(),
                || Some(0),
                |_| panic!("no raid to resolve")
            ),
            Some(false)
        );
        assert_eq!(
            store.reference(0x1000, 1),
            Some(cair_wing::transport_operands::RaidReference {
                raw: 0,
                resolves: false
            })
        );
        assert_eq!(
            store.capture_with(
                0x2000,
                2,
                |at| registry.sequence(at),
                || registry.quiet(),
                || Some(77),
                |_| Some(Some(0x9000))
            ),
            Some(true)
        );
        let calls = std::cell::Cell::new(0);
        assert_eq!(
            store.capture_with(
                0x3000,
                3,
                |at| registry.sequence(at),
                || {
                    calls.set(calls.get() + 1);
                    if calls.get() == 2 {
                        drop(registry.notify(0));
                    }
                    registry.quiet()
                },
                || Some(77),
                |_| Some(Some(0x9000))
            ),
            None,
            "a writer between the tokens"
        );
        store.finish();
        store.begin(3, false);
        assert_eq!(
            store.capture_with(
                0x1000,
                1,
                |_| Some((1, 0)),
                || Some((1, 0)),
                || Some(77),
                |_| Some(Some(0x9000))
            ),
            Some(true),
            "an uncertified read is read again, never carried"
        );
    }

    #[test]
    fn stale_observed_token_falls_back_to_live_read_instead_of_caching_none() {
        struct LiveMemory {
            wing_zero: usize,
            wing_nonzero: usize,
            raw: u64,
        }
        impl Memory for LiveMemory {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == self.wing_zero + sites::RAID_REFERENCE && output.len() == 8 {
                    output.copy_from_slice(&0_u64.to_le_bytes());
                    true
                } else if address == self.wing_nonzero + sites::RAID_REFERENCE && output.len() == 8
                {
                    output.copy_from_slice(&self.raw.to_le_bytes());
                    true
                } else if output.len() == 8 {
                    output.fill(0);
                    true
                } else {
                    false
                }
            }
        }
        let registry = collector_certificates::watch_registry::WatchRegistry::new();
        registry.offer(8);
        registry.boundary(false);
        registry.set_enabled(true);
        let raw = (1_u64 << 32) | u64::from(craid_instance::REFERENCE_TYPE_VALUE);
        let memory = LiveMemory {
            wing_zero: 0x1000,
            wing_nonzero: 0x2000,
            raw,
        };
        let current = registry.quiet().unwrap();
        let stale = Some((current.0, current.1.wrapping_add(1)));
        let row_zero = cair_wing::Row {
            id: 11,
            air_base: 1,
            raid_reference: Some(cair_wing::transport_operands::RaidReference {
                raw: 0,
                resolves: false,
            }),
            ..Default::default()
        };
        let row_nonzero = cair_wing::Row {
            id: 12,
            air_base: 1,
            raid_reference: Some(cair_wing::transport_operands::RaidReference {
                raw,
                resolves: false,
            }),
            ..Default::default()
        };
        let mut store = Store::default();
        store.begin(2, true);
        assert_eq!(
            store.capture_observed(
                &memory,
                0,
                0x1000,
                &row_zero,
                Some(0),
                stale,
                |at, slot| registry.sequence_at(at, slot),
                || registry.quiet(),
            ),
            Some(false)
        );
        assert_eq!(
            store.capture_observed(
                &memory,
                0,
                0x2000,
                &row_nonzero,
                Some(0),
                stale,
                |at, slot| registry.sequence_at(at, slot),
                || registry.quiet(),
            ),
            Some(false)
        );
        store.finish();
        store.begin(2, false);
        assert_eq!(
            store.capture_observed(
                &memory,
                0,
                0x1000,
                &row_zero,
                Some(0),
                stale,
                |at, slot| registry.sequence_at(at, slot),
                || registry.quiet(),
            ),
            Some(false)
        );
        assert_eq!(
            store.capture_observed(
                &memory,
                0,
                0x2000,
                &row_nonzero,
                Some(0),
                stale,
                |at, slot| registry.sequence_at(at, slot),
                || registry.quiet(),
            ),
            Some(false)
        );
    }
}
