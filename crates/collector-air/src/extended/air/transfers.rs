use super::{BaseWitness, WingIdentity};
use engine_allocation as allocation;
use engine_native::{Memory, cair_base, cair_wing, cpdx_array::Array, read};
use replay_data::rows::AirTransferInputRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Member {
    wing: u64,
    ordinal: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_map_air::map_air_base_reference::BaseReference;
    use engine_native::memory::SliceMemory;

    struct MemoryView<'a> {
        bytes: &'a [u8],
        deny_roster: bool,
    }
    impl Memory for MemoryView<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self.deny_roster && address < 0x200 {
                return false;
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }

    fn word(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn world(count: usize) -> (Vec<u8>, Vec<WingIdentity>, Vec<usize>, [BaseWitness; 1]) {
        let mut bytes = vec![0; 0x1000 + count * 0x100];
        word(&mut bytes, 0x40 + cair_base::INCOMING_WINGS, 0x1000);
        word(
            &mut bytes,
            0x40 + cair_base::INCOMING_WINGS + 8,
            (count as u64) | ((count as u64) << 32),
        );
        let wings: Vec<_> = (0..count)
            .map(|at| {
                let pointer = 0x1000 + count * 8 + at * 0x80;
                word(&mut bytes, 0x1000 + at * 8, pointer as u64);
                word(
                    &mut bytes,
                    pointer + cair_wing::RELOCATION_PROGRESS,
                    (at as u64 + 1) * 100_000,
                );
                word(
                    &mut bytes,
                    pointer + cair_wing::RELOCATION_DISTANCE,
                    1_000_000,
                );
                WingIdentity {
                    wing: at as u64 + 1,
                    mission_pointer: pointer + cair_wing::MISSION,
                    base_override: 77,
                    ..Default::default()
                }
            })
            .collect();
        (
            bytes,
            wings,
            (0..count).collect(),
            [BaseWitness {
                handle: 77,
                base: Some(BaseReference {
                    object: 0x40,
                    ..Default::default()
                }),
                ..Default::default()
            }],
        )
    }

    #[test]
    fn quiet_rosters_are_carried_but_only_admitted_progress_is_read() {
        let (mut bytes, mut wings, demand, bases) = world(3);
        wings[2].eligibility_counter = 1;
        let mut store = Store::default();
        let mut output = Vec::new();
        store
            .capture_with(
                &MemoryView {
                    bytes: &bytes,
                    deny_roster: false,
                },
                &wings,
                &demand,
                &bases,
                &mut output,
                |_| Some((1, 0)),
            )
            .unwrap();
        assert_eq!(
            output
                .iter()
                .map(|r| (r.wing, r.ordinal, r.progress_raw))
                .collect::<Vec<_>>(),
            [
                (1, Some(0), Some(100_000)),
                (2, Some(1), Some(200_000)),
                (3, Some(2), None)
            ]
        );
        word(
            &mut bytes,
            wings[0].mission_pointer - cair_wing::MISSION + cair_wing::RELOCATION_PROGRESS,
            125_000,
        );
        store
            .capture_with(
                &MemoryView {
                    bytes: &bytes,
                    deny_roster: true,
                },
                &wings,
                &demand,
                &bases,
                &mut output,
                |_| Some((1, 0)),
            )
            .unwrap();
        assert_eq!(output[0].ordinal, Some(0));
        assert_eq!(output[0].progress_raw, Some(125_000));
        assert_eq!(output[2].distance_raw, None);
    }

    #[test]
    fn swap_remove_order_and_failed_certified_reads_do_not_poll() {
        let (mut bytes, mut wings, demand, bases) = world(3);
        let mut store = Store::default();
        let mut output = Vec::new();
        store
            .capture_with(
                &MemoryView {
                    bytes: &bytes,
                    deny_roster: false,
                },
                &wings,
                &demand,
                &bases,
                &mut output,
                |_| Some((1, 0)),
            )
            .unwrap();
        store
            .capture_with(
                &MemoryView {
                    bytes: &bytes,
                    deny_roster: true,
                },
                &wings,
                &demand,
                &bases,
                &mut output,
                |_| Some((1, 1)),
            )
            .unwrap();
        assert!(
            output
                .iter()
                .all(|row| row.ordinal.is_none() && row.progress_raw.is_none())
        );
        store
            .capture_with(
                &MemoryView {
                    bytes: &bytes,
                    deny_roster: false,
                },
                &wings,
                &demand,
                &bases,
                &mut output,
                |_| Some((1, 1)),
            )
            .unwrap();
        assert!(output.iter().all(|row| row.ordinal.is_none()));
        wings[0].base_override = 0;
        word(
            &mut bytes,
            0x1000,
            (wings[2].mission_pointer - cair_wing::MISSION) as u64,
        );
        word(
            &mut bytes,
            0x40 + cair_base::INCOMING_WINGS + 8,
            3 | (2 << 32),
        );
        store
            .capture_with(
                &MemoryView {
                    bytes: &bytes,
                    deny_roster: false,
                },
                &wings,
                &[1, 2],
                &bases,
                &mut output,
                |_| Some((1, 2)),
            )
            .unwrap();
        assert_eq!(
            output
                .iter()
                .map(|r| (r.wing, r.ordinal))
                .collect::<Vec<_>>(),
            [(2, Some(1)), (3, Some(0))]
        );
    }

    #[test]
    fn incoming_pointers_join_the_handle_sorted_census_without_assuming_address_order() {
        let (bytes, mut wings, demand, bases) = world(2);
        wings[0].wing = 2;
        wings[1].wing = 1;
        wings.swap(0, 1);
        let mut store = Store::default();
        let mut output = Vec::new();
        store
            .capture_with(
                &MemoryView {
                    bytes: &bytes,
                    deny_roster: false,
                },
                &wings,
                &demand,
                &bases,
                &mut output,
                |_| Some((1, 0)),
            )
            .unwrap();
        assert_eq!(
            output
                .iter()
                .map(|row| (row.wing, row.ordinal, row.progress_raw))
                .collect::<Vec<_>>(),
            [(1, Some(1), Some(200_000)), (2, Some(0), Some(100_000))]
        );
    }
}

struct Base {
    handle: u64,
    object: usize,
    certificate: (u64, u64),
    known: bool,
    members: Vec<Member>,
}

#[derive(Default)]
pub struct Store {
    bases: Vec<Base>,
    demand: Vec<(u64, usize)>,
    pointers: Vec<(usize, usize)>,
    scratch: Vec<Member>,
}

impl Store {
    pub fn clear(&mut self) {
        self.bases.clear();
        self.demand.clear();
        self.pointers.clear();
        self.scratch.clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bases.capacity() * size_of::<Base>()
            + self.demand.capacity() * size_of::<(u64, usize)>()
            + self.pointers.capacity() * size_of::<(usize, usize)>()
            + self.scratch.capacity() * size_of::<Member>()
            + self
                .bases
                .iter()
                .map(|base| base.members.capacity() * size_of::<Member>())
                .sum::<usize>()
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        wings: &[WingIdentity],
        demand: &[usize],
        witnesses: &[BaseWitness],
        output: &mut Vec<AirTransferInputRow>,
    ) -> Result<(), &'static str> {
        self.capture_with(
            memory,
            wings,
            demand,
            witnesses,
            output,
            collector_observers::wing_changes::map_sequence,
        )
    }

    fn capture_with(
        &mut self,
        memory: &impl Memory,
        wings: &[WingIdentity],
        demand: &[usize],
        witnesses: &[BaseWitness],
        output: &mut Vec<AirTransferInputRow>,
        mut certificate: impl FnMut(usize) -> Option<(u64, u64)>,
    ) -> Result<(), &'static str> {
        output.clear();
        self.demand.clear();
        self.pointers.clear();
        allocation::advisory(|| {
            allocation::reserve(&mut self.demand, demand.len())?;
            allocation::reserve(&mut self.pointers, demand.len())?;
            allocation::reserve(output, demand.len())
        })
        .map_err(|_| "Air transfer demand storage unavailable")?;
        self.demand.extend(
            demand
                .iter()
                .map(|&index| (wings[index].base_override, index)),
        );
        self.pointers.extend(
            demand
                .iter()
                .map(|&index| (wings[index].mission_pointer, index)),
        );
        self.pointers.sort_unstable_by_key(|row| row.0);
        self.demand.sort_unstable();
        self.bases.retain(|base| {
            self.demand
                .binary_search_by_key(&base.handle, |row| row.0)
                .is_ok()
        });
        let count = self
            .demand
            .iter()
            .enumerate()
            .filter(|(at, row)| *at == 0 || self.demand[*at - 1].0 != row.0)
            .count();
        allocation::advisory(|| allocation::reserve(&mut self.bases, count))
            .map_err(|_| "Air transfer base storage unavailable")?;
        let mut first = 0;
        while first < self.demand.len() {
            let handle = self.demand[first].0;
            let end = self.demand[first..]
                .iter()
                .position(|row| row.0 != handle)
                .map_or(self.demand.len(), |at| first + at);
            let witness = witnesses
                .binary_search_by_key(&handle, |row| row.handle)
                .ok()
                .and_then(|at| witnesses[at].base);
            let observed = witness.and_then(|base| {
                let key = base.object.checked_add(cair_base::INCOMING_WINGS)?;
                Some((base.object, key, certificate(key)?))
            });
            let known = if let Some((object, key, before)) = observed {
                let at = match self.bases.binary_search_by_key(&handle, |base| base.handle) {
                    Ok(at) => at,
                    Err(at) => {
                        self.bases.insert(
                            at,
                            Base {
                                handle,
                                object,
                                certificate: before,
                                known: false,
                                members: Vec::new(),
                            },
                        );
                        self.read(at, memory, wings);
                        at
                    }
                };
                if self.bases[at].object != object || self.bases[at].certificate != before {
                    let base = &mut self.bases[at];
                    base.object = object;
                    base.certificate = before;
                    self.read(at, memory, wings);
                }
                if certificate(key) != Some(before) {
                    self.bases[at].known = false;
                }
                self.bases[at].known.then_some(at)
            } else {
                None
            };
            for &(_, index) in &self.demand[first..end] {
                let wing = &wings[index];
                let ordinal = known.and_then(|at| {
                    self.bases[at]
                        .members
                        .binary_search_by_key(&wing.wing, |member| member.wing)
                        .ok()
                        .map(|index| self.bases[at].members[index].ordinal)
                });
                if known.is_some() && ordinal.is_none() {
                    continue;
                }
                let pointer = wing.mission_pointer.checked_sub(cair_wing::MISSION);
                let reached =
                    ordinal.is_some() && wing.eligibility_counter <= wing.eligibility_limit;
                let value = |offset| {
                    reached
                        .then(|| {
                            pointer
                                .and_then(|pointer| pointer.checked_add(offset))
                                .and_then(|at| read::<i64>(memory, at))
                        })
                        .flatten()
                };
                output.push(AirTransferInputRow {
                    wing: wing.wing,
                    base: handle,
                    ordinal,
                    progress_raw: value(cair_wing::RELOCATION_PROGRESS),
                    distance_raw: value(cair_wing::RELOCATION_DISTANCE),
                });
            }
            first = end;
        }
        output.sort_unstable_by_key(|row| row.wing);
        Ok(())
    }

    fn read(&mut self, index: usize, memory: &impl Memory, wings: &[WingIdentity]) {
        let base = &mut self.bases[index];
        base.known = false;
        self.scratch.clear();
        let result = (|| {
            let array = base
                .object
                .checked_add(cair_base::INCOMING_WINGS)
                .and_then(|at| Array::read(memory, at, 8))
                .ok_or("Air incoming-wing header unreadable")?;
            allocation::advisory(|| allocation::reserve(&mut self.scratch, array.count as usize))
                .map_err(|_| "Air incoming-wing storage unavailable")?;
            engine_native::cpdx_array::pointers(
                memory,
                array,
                "Air incoming-wing vector unreadable",
                |ordinal, pointer| {
                    let mission = pointer
                        .checked_add(cair_wing::MISSION)
                        .ok_or("Air incoming-wing pointer overflow")?;
                    let wing = self
                        .pointers
                        .binary_search_by_key(&mission, |row| row.0)
                        .ok()
                        .map(|at| &wings[self.pointers[at].1])
                        .ok_or("Air incoming wing is absent from the complete census")?;
                    if wing.base_override != base.handle {
                        return Err("Air incoming-wing destination differs from its base vector");
                    }
                    self.scratch.push(Member {
                        wing: wing.wing,
                        ordinal,
                    });
                    Ok(())
                },
            )?;
            self.scratch.sort_unstable_by_key(|member| member.wing);
            if self
                .scratch
                .windows(2)
                .any(|pair| pair[0].wing == pair[1].wing)
            {
                return Err("Air incoming-wing vector repeats a wing");
            }
            Ok(())
        })();
        if result.is_ok() {
            std::mem::swap(&mut base.members, &mut self.scratch);
            base.known = true;
        }
    }
}
