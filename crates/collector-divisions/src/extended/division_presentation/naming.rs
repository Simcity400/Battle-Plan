use super::*;
use cname_group_member::{GroupKeys, INPUT_SPAN, Span};
use collector_division_parts::army_stat_changes::Changes;
use engine_allocation::SharedBox;

type Store = collector_division_parts::attack_stats::ArmyStatStore;

#[derive(Default)]
pub struct NamingCarry {
    carryable: bool,
    touched: Vec<usize>,
    groups: [usize; 2],
    seeds: usize,
    bytes: [usize; 2],
    hour: i32,
    held_index: Option<Vec<Option<u32>>>,
}

impl NamingCarry {
    pub(super) fn allocated_bytes(&self) -> usize {
        self.touched.capacity() * size_of::<usize>()
            + self
                .held_index
                .as_ref()
                .map_or(0, |held| held.capacity() * size_of::<Option<u32>>())
    }

    pub(super) fn stage_attack(
        &self,
        attack: &mut collector_division_parts::attack_stats::AttackPart,
        seeds: &[DivisionSeed],
        store: Option<&Store>,
        prepared: &carmy::StatusPacket,
    ) {
        match &self.held_index {
            Some(held_index) => attack.prepare(seeds, self.hour, held_index, store, |index| {
                prepared.unit_maxima(index)
            }),
            None => attack.clear(),
        }
    }

    pub(super) fn carry(&self) -> Carry<'_> {
        Carry {
            carryable: self.carryable,
            touched: &self.touched,
            groups: self.groups,
            seeds: self.seeds,
            bytes: self.bytes,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Carry<'a> {
    pub carryable: bool,
    pub touched: &'a [usize],
    pub groups: [usize; 2],
    pub seeds: usize,
    pub bytes: [usize; 2],
}

struct Keep {
    index: usize,
    division: u64,
    member: usize,
    span: [u8; INPUT_SPAN],
    text: Span,
    stamp: u64,
}

pub struct NamingPart {
    pub(super) range: std::ops::Range<usize>,
    rows: Vec<NamingRead>,
    len: usize,
    names: Arena<u8>,
    cached: Arena<u8>,
    groups: GroupKeys,
    keeps: Vec<Keep>,
    lookups: usize,
    failed: bool,
}

impl Default for NamingPart {
    fn default() -> Self {
        Self::with_groups(GroupKeys::default())
    }
}

impl NamingPart {
    pub(super) fn with_groups(groups: GroupKeys) -> Self {
        Self {
            range: 0..0,
            rows: Vec::new(),
            len: 0,
            names: Arena::new(0),
            cached: Arena::new(0),
            groups,
            keeps: Vec::new(),
            lookups: 0,
            failed: true,
        }
    }

    pub(super) fn groups(&self) -> &GroupKeys {
        &self.groups
    }

    pub(super) fn set_groups(&mut self, groups: GroupKeys) {
        self.groups = groups;
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<NamingRead>()
            + self.names.allocated_bytes()
            + self.cached.allocated_bytes()
            + self.groups.allocated_bytes()
            + self.keeps.capacity() * size_of::<Keep>()
    }

    pub(super) fn arm(&mut self, range: std::ops::Range<usize>) {
        self.range = range;
        self.failed = true;
    }

    pub(super) fn fail(&mut self) {
        self.failed = true;
    }

    #[cfg(test)]
    pub(super) fn forget_staging(&mut self) {
        self.keeps = Vec::new();
    }

    #[cfg(test)]
    pub(super) fn named(&self) -> Option<usize> {
        (!self.failed).then_some(self.len)
    }

    pub(super) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        carry: Carry<'_>,
        store: Option<&Store>,
        changes: &dyn Changes,
    ) {
        self.len = 0;
        self.names.reset();
        self.cached.reset();
        self.keeps.clear();
        self.lookups = 0;
        self.failed = false;
        let first = self.range.start.min(seeds.len());
        let seeds = &seeds[first..self.range.end.clamp(first, seeds.len())];
        let share = |total: usize| {
            total
                .saturating_mul(2)
                .saturating_mul(seeds.len())
                .div_ceil(carry.seeds.max(1))
                .min(total)
        };
        let group_target = |total: usize| {
            if total == 0 {
                0
            } else {
                share(total)
                    .max(64)
                    .min(total)
                    .checked_next_power_of_two()
                    .unwrap_or(total)
            }
        };
        let target_groups = [group_target(carry.groups[0]), group_target(carry.groups[1])];
        if self.groups.capacities() != target_groups
            && let Ok(groups) = GroupKeys::try_with_capacity(target_groups[0], target_groups[1])
                .map_err(collector_memory::terminal::io)
        {
            self.groups = groups;
        }
        self.groups.reset();
        if !grow_vec(&mut self.rows, seeds.len())
            || engine_allocation::reserve(&mut self.keeps, seeds.len()).is_err()
        {
            self.failed = true;
            return;
        }
        engine_allocation::advisory(|| {
            grow_vec(&mut self.names.first, share(carry.bytes[0]));
            grow_vec(&mut self.cached.first, share(carry.bytes[1]));
        });
        let mut shift = 0_isize;
        let mut batch = cname_group_member::MemberBatch::default();
        let mut held_indices = [None; 64];
        for (local, (seed, row)) in seeds.iter().zip(&mut self.rows).enumerate() {
            let index = first + local;
            if local.is_multiple_of(64) {
                held_indices.fill(None);
                let mut pointers = [0_usize; 64];
                let end = (local + 64).min(seeds.len());
                for (slot, pending) in seeds[local..end].iter().enumerate() {
                    let Some(pointer) = pending.name_member.filter(|&at| at != 0) else {
                        continue;
                    };
                    let held = match store {
                        Some(store) if carry.carryable => {
                            self.lookups += 1;
                            store
                                .find_held_name(&mut shift, index + slot, pending.division)
                                .filter(|&at| {
                                    store.held_name_record(at).member == pointer
                                        && carry.touched.binary_search(&pointer).is_err()
                                })
                        }
                        _ => None,
                    };
                    held_indices[slot] = held;
                    if held.is_none() {
                        pointers[slot] = pointer;
                    }
                }
                batch.capture(memory, &pointers[..end - local]);
            }
            *row = NamingRead {
                division: seed.division,
                country_ordinal: seed.country_ordinal,
                ..NamingRead::default()
            };
            let pointer = seed
                .name_member
                .ok_or("division name member pointer unreadable")
                .and_then(|pointer| {
                    if pointer == 0 {
                        Err("division name member pointer null")
                    } else {
                        Ok(pointer)
                    }
                });
            let held = held_indices[local % 64]
                .and_then(|at| store.map(|store| store.held_name_record(at)));
            let registered = match (held, pointer, store.is_some()) {
                (Some(_), _, _) => true,
                (None, Ok(pointer), true) => changes.register(pointer),
                _ => false,
            };
            let member = pointer.map(|pointer| match held {
                Some(held) => (pointer, Ok(held.span)),
                None => (pointer, batch.member(memory, local % 64, pointer)),
            });
            let result = member.and_then(|(pointer, span)| {
                cname_group_member::inputs_growing(
                    memory,
                    base,
                    pointer,
                    &span?,
                    &mut self.groups,
                    &mut self.names.first,
                    &mut self.names.len,
                    &mut self.names.required,
                )
            });
            match result {
                Ok(cname_group_member::Naming::Row(inputs)) => {
                    self.names.len += inputs.custom.1;
                    row.inputs = Some(inputs);
                }
                Ok(cname_group_member::Naming::DoesNotFit) => {
                    row.failure = Some("division naming bytes do not fit arena");
                }
                Err(reason) => row.failure = Some(reason),
            }
            let cached = match (member, held, store) {
                (_, Some(held), Some(store)) => cname_group_member::held_cached_name(
                    store.held_name_text(held),
                    &mut self.cached.first,
                    self.cached.len,
                    &mut self.cached.required,
                ),
                (Ok((_, Ok(span))), _, _) => cname_group_member::cached_name_from_member(
                    memory,
                    base,
                    &span,
                    &mut self.cached.first,
                    &mut self.cached.len,
                    &mut self.cached.required,
                ),
                _ => seed
                    .name_member
                    .ok_or("division cached name member pointer unreadable")
                    .and_then(|pointer| {
                        cname_group_member::cached_name_growing(
                            memory,
                            base,
                            pointer,
                            &mut self.cached.first,
                            &mut self.cached.len,
                            &mut self.cached.required,
                        )
                    }),
            };
            match cached {
                Ok(Some(span)) => {
                    row.cached_name = Some(span);
                    self.cached.len += span.1;
                }
                Ok(None) => {
                    row.cached_failure = Some("division cached name bytes do not fit arena")
                }
                Err(reason) => row.cached_failure = Some(reason),
            }
            row.held = held.is_some();
            let stamp = held.map_or(0, |held| held.stamp);
            row.held_stamp = match (held, row.inputs, row.cached_name) {
                (Some(held), Some(_), Some(_)) => held.stamp,
                _ => 0,
            };
            if let (true, true, Ok((pointer, Ok(span))), Some(text)) =
                (store.is_some(), registered, member, row.cached_name)
            {
                self.keeps.push(Keep {
                    index,
                    division: seed.division,
                    member: pointer,
                    span,
                    text,
                    stamp,
                });
            }
            self.len += 1;
        }
    }
}

impl Captured {
    pub(super) fn arm_naming(
        &mut self,
        admitted: usize,
        count: usize,
        hour: i32,
    ) -> Result<(), &'static str> {
        if self
            .naming_carry
            .as_mut()
            .is_none_or(|carry| SharedBox::get_mut(carry).is_none())
        {
            self.naming_carry = Some(
                engine_allocation::shared(NamingCarry::default())
                    .map_err(|_| "division naming carry allocation failed")?,
            );
        }
        let bytes = self.naming_bytes();
        let carry = self
            .naming_carry
            .as_mut()
            .and_then(SharedBox::get_mut)
            .expect("naming carry unshared");
        carry.groups = self.serial_naming.groups.capacities();
        carry.seeds = admitted;
        carry.bytes = bytes;
        carry.touched.clear();
        let touched = self.attack.touched_receivers();
        carry.carryable = self.attack.carryable
            && engine_allocation::reserve(&mut carry.touched, touched.len()).is_ok();
        if carry.carryable {
            carry.touched.extend_from_slice(touched);
        }
        carry.hour = hour;
        let held_index = self.attack.held_index();
        let mut staged = carry.held_index.take().unwrap_or_default();
        staged.clear();
        carry.held_index = engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut staged, held_index.len())
        })
        .is_ok()
        .then(|| {
            staged.extend_from_slice(held_index);
            staged
        });
        let additional = count.saturating_sub(self.naming_parts.len());
        engine_allocation::reserve(&mut self.naming_parts, additional)
            .map_err(|_| "division naming part allocation failed")?;
        while self.naming_parts.len() < count {
            self.naming_parts.push(NamingPart::default());
        }
        for (part, naming) in self.naming_parts[..count].iter_mut().enumerate() {
            naming.arm(admitted * part / count..admitted * (part + 1) / count);
        }
        Ok(())
    }

    pub(super) fn naming_carry(&self) -> SharedBox<NamingCarry> {
        SharedBox::clone(self.naming_carry.as_ref().expect("naming armed"))
    }

    fn naming_bytes(&self) -> [usize; 2] {
        [
            self.names.first.len().max(self.names.required),
            self.cached_names.first.len().max(self.cached_names.required),
        ]
    }

    pub(super) fn capture_naming_holding(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        store: Option<&mut Store>,
        changes: &dyn Changes,
    ) {
        let mut part = std::mem::take(&mut self.serial_naming);
        part.range = 0..seeds.len();
        part.capture(
            memory,
            base,
            seeds,
            Carry {
                carryable: self.attack.carryable,
                touched: self.attack.touched_receivers(),
                groups: part.groups.capacities(),
                seeds: seeds.len(),
                bytes: self.naming_bytes(),
            },
            store.as_deref(),
            changes,
        );
        #[cfg(test)]
        let mut store = store;
        #[cfg(test)]
        if let Some(store) = store.as_deref_mut() {
            store.name_lookups += part.lookups;
        }
        self.stitch_naming(std::slice::from_mut(&mut part), seeds.len(), store);
        self.serial_naming = part;
    }

    pub(super) fn stitch_naming(
        &mut self,
        parts: &mut [NamingPart],
        len: usize,
        mut store: Option<&mut Store>,
    ) {
        self.naming.reset();
        self.names.reset();
        self.cached_names.reset();
        if let Some(store) = store.as_deref_mut() {
            store.begin_names();
        }
        self.naming.required = len;
        let contiguous = parts
            .iter()
            .scan(0, |next, part| {
                let joins = part.range.start == *next;
                *next = part.range.end;
                Some(joins)
            })
            .all(|joins| joins);
        if !contiguous || parts.iter().any(|part| part.failed) {
            return;
        }
        let Some((first, rest)) = parts.split_first_mut() else {
            return;
        };
        std::mem::swap(&mut self.naming.first, &mut first.rows);
        std::mem::swap(&mut self.names.first, &mut first.names.first);
        std::mem::swap(&mut self.cached_names.first, &mut first.cached.first);
        self.naming.len = first.len.min(len);
        self.names.len = first.names.len;
        self.names.required = first.names.required;
        self.cached_names.len = first.cached.len;
        self.cached_names.required = first.cached.required;
        let names = rest.iter().map(|part| part.names.len).sum::<usize>();
        let cached = rest.iter().map(|part| part.cached.len).sum::<usize>();
        if !grow_vec(&mut self.naming.first, len)
            || !grow_vec(&mut self.names.first, self.names.len + names)
            || !grow_vec(&mut self.cached_names.first, self.cached_names.len + cached)
        {
            self.naming.len = 0;
            return;
        }
        for part in rest.iter() {
            let (names_at, cached_at) = (self.names.len, self.cached_names.len);
            self.names.first[names_at..names_at + part.names.len]
                .copy_from_slice(&part.names.first[..part.names.len]);
            self.cached_names.first[cached_at..cached_at + part.cached.len]
                .copy_from_slice(&part.cached.first[..part.cached.len]);
            self.names.len += part.names.len;
            self.names.required += part.names.required;
            self.cached_names.len += part.cached.len;
            self.cached_names.required += part.cached.required;
            for row in &part.rows[..part.len] {
                if self.naming.len >= len {
                    break;
                }
                let mut row = *row;
                if let Some(inputs) = row.inputs.as_mut() {
                    inputs.custom.0 += names_at;
                    inputs.group.0 += names_at;
                }
                if let Some(span) = row.cached_name.as_mut() {
                    span.0 += cached_at;
                }
                self.naming.first[self.naming.len] = row;
                self.naming.len += 1;
            }
        }
        self.attack.status_counts.names_reused += self.naming.first[..self.naming.len]
            .iter()
            .filter(|row| row.held)
            .count() as u64;
        if let Some(store) = store {
            for keep in first.keeps.iter().take_while(|keep| keep.index < len) {
                let text = &self.cached_names.first[keep.text.0..keep.text.0 + keep.text.1];
                store.keep_name(keep.division, keep.member, &keep.span, text, keep.stamp);
            }
            for part in rest.iter() {
                for keep in part.keeps.iter().take_while(|keep| keep.index < len) {
                    let text = &part.cached.first[keep.text.0..keep.text.0 + keep.text.1];
                    store.keep_name(keep.division, keep.member, &keep.span, text, keep.stamp);
                }
            }
        }
    }
}
