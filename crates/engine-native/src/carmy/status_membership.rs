use super::*;
use engine_native_memory::certified_read::{Certified, Decision};
use crate::corders_group_member::MembershipSignals;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    base: usize,
    division: u64,
    query: MembershipQuery,
    authenticated: bool,
}
#[derive(PartialEq, Eq)]
struct Certificate {
    key: Key,
    own: (u64, u64),
    parent: Option<(usize, (u64, u64))>,
}
#[derive(Default)]
struct Value {
    parent: Option<usize>,
    has_order: bool,
}
struct Slots {
    own: usize,
    parent: usize,
}
impl Default for Slots {
    fn default() -> Self {
        Self {
            own: usize::MAX,
            parent: usize::MAX,
        }
    }
}
struct Leaf {
    value: Certified<Value, Certificate>,
    slots: Slots,
}
impl Default for Leaf {
    fn default() -> Self {
        Self {
            value: Certified::new(Value::default()),
            slots: Slots::default(),
        }
    }
}
fn signal(
    signals: &impl MembershipSignals,
    key: Key,
    value: &Value,
    slots: &mut Slots,
) -> Option<Certificate> {
    let (own, slot) = signals.group(key.query.own_group, slots.own)?;
    slots.own = slot;
    let parent = match value.parent? {
        0 => None,
        receiver => {
            let (stamp, slot) = signals.group(receiver, slots.parent)?;
            slots.parent = slot;
            Some((receiver, stamp))
        }
    };
    Some(Certificate { key, own, parent })
}

struct Entry {
    division: u64,
    taken: bool,
    leaf: Leaf,
}

fn take_leaf(
    leaves: &mut [Entry],
    lookup: &mut Vec<(u64, u32)>,
    lookup_built: &mut bool,
    shift: &mut isize,
    index: usize,
    division: u64,
) -> Leaf {
    if let Some(at) = index.checked_add_signed(*shift)
        && let Some(entry) = leaves.get_mut(at)
        && !entry.taken
        && entry.division == division
    {
        entry.taken = true;
        return std::mem::take(&mut entry.leaf);
    }
    for delta in [1isize, -1, 2, -2, 3, -3, 4, -4] {
        if let Some(at) = index.checked_add_signed(*shift + delta)
            && let Some(entry) = leaves.get_mut(at)
            && !entry.taken
            && entry.division == division
        {
            entry.taken = true;
            *shift = at as isize - index as isize;
            return std::mem::take(&mut entry.leaf);
        }
    }
    if !*lookup_built && !leaves.is_empty() {
        *lookup_built = true;
        let ok = crate::allocation::advisory(|| {
            if lookup.capacity() < leaves.len() {
                crate::allocation::reserve(lookup, leaves.len())
            } else {
                Ok(())
            }
        })
        .is_ok();
        if ok {
            lookup.extend(
                leaves
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| !entry.taken)
                    .map(|(at, entry)| (entry.division, at as u32)),
            );
            lookup.sort_unstable_by_key(|entry| entry.0);
        }
    }
    if let Ok(pos) = lookup.binary_search_by_key(&division, |entry| entry.0) {
        let at = lookup[pos].1 as usize;
        let entry = &mut leaves[at];
        if !entry.taken && entry.division == division {
            entry.taken = true;
            *shift = at as isize - index as isize;
            return std::mem::take(&mut entry.leaf);
        }
    }
    Leaf::default()
}

pub(super) struct Cache {
    leaves: Vec<Entry>,
    next: Vec<Entry>,
    lookup: Vec<(u64, u32)>,
    #[cfg(feature = "order_membership_oracle")]
    oracle: MembershipPacket,
}
impl Default for Cache {
    fn default() -> Self {
        Self {
            leaves: Vec::new(),
            next: Vec::new(),
            lookup: Vec::new(),
            #[cfg(feature = "order_membership_oracle")]
            oracle: MembershipPacket::vacant(),
        }
    }
}
impl Cache {
    pub fn allocated_bytes(&self) -> usize {
        let bytes = (self.leaves.capacity() + self.next.capacity()) * std::mem::size_of::<Entry>()
            + self.lookup.capacity() * std::mem::size_of::<(u64, u32)>();
        #[cfg(feature = "order_membership_oracle")]
        let bytes = bytes + self.oracle.allocated_bytes();
        bytes
    }
    pub fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        packet: &mut StatusPacket,
        signals: &impl MembershipSignals,
    ) -> Result<(), &'static str> {
        #[cfg(feature = "order_membership_oracle")]
        let before = measure();
        let count = packet
            .units
            .iter()
            .position(|unit| unit.failure.is_some())
            .unwrap_or(packet.units.len());
        let mut available = signals.available();
        self.next.clear();
        self.lookup.clear();
        if available {
            available = engine_allocation::advisory(|| {
                if self.next.capacity() < count {
                    engine_allocation::reserve(&mut self.next, count)
                } else {
                    Ok(())
                }
            })
            .is_ok();
        }
        let key_at = |index: usize, unit: &StatusUnitInputs| -> Result<Key, &'static str> {
            Ok(Key {
                base,
                division: seeds[index].division,
                query: MembershipQuery {
                    member: seeds[index]
                        .pointer
                        .checked_add(cunit::SCHEDULED_MEMBER)
                        .ok_or("division membership address overflow")?,
                    own_group: unit.group,
                },
                authenticated: super::status_parts::census_authenticated(&seeds[index]),
            })
        };
        let mut lookup_built = false;
        let mut shift: isize = 0;
        let leaves = &mut self.leaves;
        let next = &mut self.next;
        let lookup = &mut self.lookup;
        for index in 0..count {
            let key = key_at(index, &packet.units[index])?;
            packet.units[index].has_order = None;
            packet.units[index].membership = usize::MAX;
            if available {
                let mut leaf = take_leaf(
                    leaves,
                    lookup,
                    &mut lookup_built,
                    &mut shift,
                    index,
                    key.division,
                );
                let carried = match leaf
                    .value
                    .begin(|value| signal(signals, key, value, &mut leaf.slots))
                {
                    Decision::Carried(value) => Some(value.has_order),
                    Decision::Read => None,
                };
                next.push(Entry {
                    division: key.division,
                    taken: false,
                    leaf,
                });
                if let Some(has_order) = carried {
                    packet.units[index].has_order = Some(Ok(has_order));
                    #[cfg(feature = "order_membership_oracle")]
                    CARRIED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }
            }
            if key.authenticated {
                let _ = packet.membership.admit_group(key.query.own_group);
            }
            let query = packet.membership.push_query(key.query)?;
            packet.units[index].membership = query;
        }
        packet.membership.acquire(memory, base);
        for index in 0..count {
            if packet.units[index].has_order.is_some() {
                continue;
            }
            let query = packet.units[index].membership;
            let outcome = packet.membership.outcome(query);
            packet.units[index].has_order = Some(outcome);
            if available {
                let key = key_at(index, &packet.units[index])?;
                let parent = packet.membership.parent(query);
                let leaf = &mut next[index].leaf;
                let _ = leaf.value.finish(
                    |value| signal(signals, key, value, &mut leaf.slots),
                    |value| {
                        let has_order = outcome?;
                        let parent = parent.ok_or("status membership parent unavailable")?;
                        *value = Value {
                            parent: Some(parent),
                            has_order,
                        };
                        Ok::<(), &'static str>(())
                    },
                );
            }
        }
        if available {
            std::mem::swap(&mut self.leaves, &mut self.next);
            self.next.clear();
        }
        #[cfg(feature = "order_membership_oracle")]
        {
            let candidate = measure();
            self.oracle.clear();
            for index in 0..count {
                let key = key_at(index, &packet.units[index])?;
                if key.authenticated {
                    let _ = self.oracle.admit_group(key.query.own_group);
                }
                self.oracle.push_query(key.query)?;
            }
            self.oracle.acquire(memory, base);
            let reference = measure();
            record(before, candidate, reference);
            for index in 0..count {
                let actual = packet.units[index]
                    .has_order
                    .ok_or("membership oracle result absent")?;
                if actual != self.oracle.outcome(index) {
                    FAILED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return Err("membership writer oracle mismatch");
                }
                VERIFIED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        Ok(())
    }
}

#[cfg(feature = "order_membership_oracle")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "order_membership_oracle")]
static VERIFIED: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "order_membership_oracle")]
static FAILED: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "order_membership_oracle")]
static CARRIED: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "order_membership_oracle")]
static WORK: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
#[cfg(feature = "order_membership_oracle")]
fn measure() -> [u64; 3] {
    let counts = engine_native_memory::memory::ReadCounts::take_current();
    counts.add_current();
    [counts.bytes, counts.spans, engine_allocation::growths()]
}
#[cfg(feature = "order_membership_oracle")]
fn record(before: [u64; 3], candidate: [u64; 3], reference: [u64; 3]) {
    for index in 0..3 {
        WORK[index].fetch_add(
            candidate[index].saturating_sub(before[index]),
            Ordering::Relaxed,
        );
        WORK[index + 3].fetch_add(
            reference[index].saturating_sub(candidate[index]),
            Ordering::Relaxed,
        );
    }
}
#[cfg(feature = "order_membership_oracle")]
pub fn oracle() -> [u64; 9] {
    let work = WORK.each_ref().map(|value| value.load(Ordering::Relaxed));
    [
        VERIFIED.load(Ordering::Relaxed),
        FAILED.load(Ordering::Relaxed),
        CARRIED.load(Ordering::Relaxed),
        work[0],
        work[1],
        work[2],
        work[3],
        work[4],
        work[5],
    ]
}
