//! Original-unit borrower for raw weight and the coastal predicate. Borrow
//! selected unit originals before their owner retires them, then retire every
//! pointer memo before the authentic callback finishes. No unit UI state.
//!
//! The weight getter reads stat+2B8. The coastal predicate walks real
//! composition+A8/+B4 with stride16 and tests member+5C0 or a transport
//! classifier's signed +5A4 equal to 0x2F9C. These are stored operands, never
//! an infantry/support reconstruction.

use crate::{Memory, cpdx_array::Array, read, selected_unit_originals::OriginalObservation};

pub type Fact<T> = Result<T, &'static str>;
const WEIGHT: usize = crate::csub_unit_definition::TRANSPORT_WEIGHT;
// Composition-relative +A8/+B4 pairs, NOT wrapper-relative +A8 stride8.
const GROUPED_MEMBERS: usize = 0xa8;
const STRIDE: usize = 16;
// The coastal predicate reads +0x5C0 first, then tests signed +0x5A4 against 0x2F9C.
const COASTAL_FLAG: usize = crate::csub_unit_definition::CAN_EXFILTRATE_FROM_COAST;
const CATEGORY_TOKEN: usize = 0x5a4;
const COASTAL_ALTERNATIVE_TOKEN: i32 = 0x2f9c;

#[derive(Debug, Clone, Copy)]
struct TemplateMemo {
    wrapper: usize,
    handle: u64,
    value: Fact<bool>,
}
#[derive(Debug, Clone, Copy)]
struct MemberMemo {
    pointer: usize,
    value: Fact<bool>,
}

impl Default for TemplateMemo {
    fn default() -> Self {
        Self {
            wrapper: 0,
            handle: 0,
            value: Err("selected transport template unobserved"),
        }
    }
}
impl Default for MemberMemo {
    fn default() -> Self {
        Self {
            pointer: 0,
            value: Err("selected transport member unobserved"),
        }
    }
}

/// Caller-owned arenas are sized off-thread from admitted live populations.
/// Successful and failed observations are memoized for this pass only. A
/// capacity refusal is sticky in its domain, so lack of memo room cannot cause
/// repeated reads. Known memo entries remain usable after a capacity refusal.
struct Memo<'a> {
    templates: crate::walk::Buffer<'a, TemplateMemo>,
    template_len: usize,
    template_capacity_failure: Option<&'static str>,
    members: crate::walk::Buffer<'a, MemberMemo>,
    member_len: usize,
    member_capacity_failure: Option<&'static str>,
    pub required_templates: usize,
    pub required_members: usize,
}

impl<'a> Memo<'a> {
    pub fn new(templates: &'a mut [TemplateMemo], members: &'a mut [MemberMemo]) -> Self {
        Self {
            templates: crate::walk::Buffer::Fixed(templates),
            template_len: 0,
            template_capacity_failure: None,
            members: crate::walk::Buffer::Fixed(members),
            member_len: 0,
            member_capacity_failure: None,
            required_templates: 0,
            required_members: 0,
        }
    }

    fn growing(templates: &'a mut Vec<TemplateMemo>, members: &'a mut Vec<MemberMemo>) -> Self {
        Self {
            templates: crate::walk::Buffer::Growing(templates),
            members: crate::walk::Buffer::Growing(members),
            template_len: 0,
            member_len: 0,
            template_capacity_failure: None,
            member_capacity_failure: None,
            required_templates: 0,
            required_members: 0,
        }
    }

    /// The original template certificate owns authentication; this leaf never
    /// rereads its selector, vtable or full handle. Weight is independently
    /// memoized by Packet over the original authenticated stat pointer.
    fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        original: &OriginalObservation,
    ) -> Fact<bool> {
        original.template.and_then(|template| {
            if let Some(entry) = self.templates[..self.template_len]
                .iter()
                .find(|entry| entry.wrapper == template.wrapper)
            {
                return if entry.handle == template.handle {
                    entry.value
                } else {
                    Err("selected transport original template identity changed in pass")
                };
            }
            if let Some(reason) = self.template_capacity_failure {
                return Err(reason);
            }
            self.required_templates = self
                .required_templates
                .checked_add(1)
                .ok_or("selected transport template demand overflow")?;
            self.templates.ensure(self.template_len + 1)?;
            if self.template_len == self.templates.len() {
                let reason = "selected transport template arena exhausted";
                self.template_capacity_failure = Some(reason);
                return Err(reason);
            }
            let value = template
                .composition()
                .and_then(|composition| self.coastal(memory, base, composition));
            self.templates[self.template_len] = TemplateMemo {
                wrapper: template.wrapper,
                handle: template.handle,
                value,
            };
            self.template_len += 1;
            value
        })
    }

    fn coastal(&mut self, memory: &impl Memory, base: usize, composition: usize) -> Fact<bool> {
        let header = at(composition, GROUPED_MEMBERS)?;
        let members = Array::read(memory, header, STRIDE)
            .ok_or("selected transport grouped composition invalid")?;
        // This is a live upper bound even when native early-false stops reads.
        self.required_members = self
            .required_members
            .checked_add(members.count as usize)
            .ok_or("selected transport grouped-member demand overflow")?;
        self.members.ensure(self.required_members)?;
        for index in 0..members.count {
            let address = members
                .element(index, STRIDE)
                .ok_or("selected transport grouped member address overflow")?;
            let pointer = read::<u64>(memory, address)
                .and_then(|pointer| usize::try_from(pointer).ok())
                .ok_or("selected transport grouped member pointer unreadable")?;
            if !self.member(memory, base, pointer)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn member(&mut self, memory: &impl Memory, base: usize, pointer: usize) -> Fact<bool> {
        if let Some(entry) = self.members[..self.member_len]
            .iter()
            .find(|entry| entry.pointer == pointer)
        {
            return entry.value;
        }
        if let Some(reason) = self.member_capacity_failure {
            return Err(reason);
        }
        if self.member_len == self.members.len() {
            let reason = "selected transport member arena exhausted";
            self.member_capacity_failure = Some(reason);
            return Err(reason);
        }
        let value = (|| {
            if pointer == 0 {
                return Err("selected transport grouped member is null");
            }
            let expected =
                base.checked_add(crate::csub_unit_definition::VTABLE_RVA)
                    .ok_or("selected transport member module overflow")? as u64;
            if read::<u64>(memory, pointer) != Some(expected) {
                return Err("selected transport grouped member identity refused");
            }
            let flag = read::<u8>(memory, at(pointer, COASTAL_FLAG)?)
                .ok_or("selected transport grouped coastal flag unreadable")?;
            if flag != 0 {
                return Ok(true);
            }
            let token = read::<i32>(memory, at(pointer, CATEGORY_TOKEN)?)
                .ok_or("selected transport grouped category word unreadable")?;
            // Preserve native opaque equality; do not invent lexical role/name.
            Ok(token == COASTAL_ALTERNATIVE_TOKEN)
        })();
        self.members[self.member_len] = MemberMemo { pointer, value };
        self.member_len += 1;
        value
    }
}

impl Drop for Memo<'_> {
    fn drop(&mut self) {
        self.templates[..self.template_len].fill(TemplateMemo::default());
        self.members[..self.member_len].fill(MemberMemo::default());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    Absent,
    Present {
        handle: u64,
        status: Fact<i32>,
        source: Fact<u32>,
    },
}

/// Transport status uses the original unit +0x2F8 selector, then the signed
/// combat reference count at +0x24. Only zero count reaches saved province
/// ID +0x54. The count is saved with token 0x2916. Refusal retains identity;
/// never retry an original selector failure.
pub(crate) fn transfer(
    memory: &impl Memory,
    base: usize,
    pointer: Fact<usize>,
    capture_source: bool,
    mut resolve: impl FnMut(u64) -> Fact<Option<usize>>,
) -> Fact<Transfer> {
    let pointer = pointer?;
    if pointer == 0 {
        return Ok(Transfer::Absent);
    }
    let expected = base
        .checked_add(crate::cnaval_unit_transfer::VTABLE_RVA)
        .ok_or("selected transport transfer module overflow")? as u64;
    if read::<u64>(memory, pointer) != Some(expected) {
        return Err("selected transport transfer vtable mismatch");
    }
    let handle = read::<u64>(memory, at(pointer, crate::cnaval_unit_transfer::HANDLE)?)
        .filter(|handle| *handle >> 32 != 0)
        .ok_or("selected transport transfer full handle invalid")?;
    if resolve(handle)? != Some(pointer) {
        return Err("selected transport transfer registry mismatch");
    }
    let status = at(pointer, crate::cnaval_unit_transfer::ACTION_STATUS).and_then(|address| {
        read(memory, address).ok_or("selected transport transfer status unreadable")
    });
    let source = if capture_source && status == Ok(0) {
        at(pointer, crate::cnaval_unit_transfer::SOURCE_PROVINCE).and_then(|address| {
            read(memory, address).ok_or("selected transport transfer source unreadable")
        })
    } else {
        Err("selected transport transfer source unreached")
    };
    Ok(Transfer::Present {
        handle,
        status,
        source,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub unit: u64,
    pub sample_hour: i32,
    pub weight: Fact<i64>,
    pub transfer: Fact<Transfer>,
    pub coastal: Fact<bool>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacity {
    pub rows: usize,
    pub members: usize,
}

/// Live-sized owned scalar output. Native memo keys die before capture returns;
/// the row buffer contains no pointers and can be decoded off-thread.
pub struct Packet {
    rows: Vec<Row>,
    templates: Vec<TemplateMemo>,
    members: Vec<MemberMemo>,
    /// Per-pass first attempts keyed by the original transfer and stat
    /// pointers; one lookup per division instead of a scan of every earlier
    /// division's entry.
    transfers: crate::combat_sources::Map<usize, Fact<Transfer>>,
    weights: crate::combat_sources::Map<usize, Fact<i64>>,
    /// Memo lookups since the last clear: one per hashed probe.
    #[cfg(test)]
    probes: usize,
    required: Capacity,
    outcome: crate::walk::Outcome,
    growing: bool,
}

impl Packet {
    pub fn vacant() -> Self {
        Self {
            rows: Vec::new(),
            templates: Vec::new(),
            members: Vec::new(),
            transfers: Default::default(),
            weights: Default::default(),
            #[cfg(test)]
            probes: 0,
            required: Capacity::default(),
            growing: false,
            outcome: crate::walk::Outcome::Unavailable {
                reason: "selected transport units not acquired",
            },
        }
    }
    pub fn enable_growth(&mut self) {
        self.growing = true;
    }
    pub fn try_new(capacity: Capacity) -> std::io::Result<Self> {
        let mut packet = Self::vacant();
        packet.reserve(capacity)?;
        Ok(packet)
    }
    pub fn reserve(&mut self, capacity: Capacity) -> std::io::Result<()> {
        fn reserve<T>(values: &mut Vec<T>, count: usize) -> std::io::Result<()> {
            if values.capacity() < count {
                crate::allocation::reserve_exact(values, count.saturating_sub(values.len()))?;
            }
            Ok(())
        }
        reserve(&mut self.rows, capacity.rows)?;
        let additional = capacity.rows.saturating_sub(self.transfers.len());
        crate::allocation::reserve_map(&mut self.transfers, additional)?;
        let additional = capacity.rows.saturating_sub(self.weights.len());
        crate::allocation::reserve_map(&mut self.weights, additional)?;
        let rows = capacity.rows.max(self.templates.len());
        let members = capacity.members.max(self.members.len());
        crate::allocation::resize(&mut self.templates, rows, TemplateMemo::default())?;
        crate::allocation::resize(&mut self.members, members, MemberMemo::default())
    }
    pub fn clear(&mut self) {
        self.rows.clear();
        self.transfers.clear();
        self.weights.clear();
        #[cfg(test)]
        {
            self.probes = 0;
        }
        self.templates.fill(TemplateMemo::default());
        self.members.fill(MemberMemo::default());
        self.required = Capacity::default();
        self.outcome = crate::walk::Outcome::Unavailable {
            reason: "selected transport units not acquired",
        };
    }
    /// Interrupted acquisition never certifies stale rows. Preserve already
    /// observed pressure so the next off-thread growth can repair a shortage.
    pub fn fail(&mut self, reason: &'static str) {
        let required = self.required;
        self.clear();
        self.required = required;
        self.outcome = crate::walk::Outcome::Unavailable { reason };
    }
    pub fn required(&self) -> Capacity {
        self.required
    }
    pub fn outcome(&self) -> crate::walk::Outcome {
        self.outcome
    }
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
    pub fn allocated_bytes(&self) -> usize {
        use std::mem::size_of;
        self.rows.capacity() * size_of::<Row>()
            + self.templates.capacity() * size_of::<TemplateMemo>()
            + self.members.capacity() * size_of::<MemberMemo>()
            + self.transfers.capacity() * (size_of::<(usize, Fact<Transfer>)>() + 1)
            + self.weights.capacity() * (size_of::<(usize, Fact<i64>)>() + 1)
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        seeds: &[crate::carmy::DivisionSeed],
        source: crate::walk::Outcome,
        originals: &crate::selected_unit_originals::Packet,
    ) {
        self.clear();
        self.required.rows = match source {
            crate::walk::Outcome::Truncated { len, unread_rows } => {
                len.saturating_add(unread_rows as usize)
            }
            _ => source.len(),
        };
        let result = (|| {
            let crate::walk::Outcome::Rows { len } = source else {
                return Err("selected transport unit census incomplete");
            };
            let seeds = seeds
                .get(..len)
                .ok_or("selected transport unit census length invalid")?;
            if self.growing {
                self.reserve(Capacity {
                    rows: len,
                    members: self.members.len(),
                })
                .map_err(|_| "selected transport allocation failed")?;
            }
            if len > self.rows.capacity()
                || len > self.templates.len()
                || len > self.transfers.capacity()
                || len > self.weights.capacity()
            {
                return Err("selected transport unit arena exhausted");
            }
            let mut memo = if self.growing {
                Memo::growing(&mut self.templates, &mut self.members)
            } else {
                Memo::new(&mut self.templates, &mut self.members)
            };
            let mut registry = crate::creference_registry::ScopedResolver::new(base);
            for seed in seeds {
                let identity = crate::selected_unit_originals::authenticate_original(seed, base);
                let original = identity.and_then(|_| {
                    let original = originals
                        .original(seed.division)
                        .ok_or("selected transport original unit observation unavailable")?;
                    if original.sample_hour != hour {
                        return Err("selected transport original sample is stale");
                    }
                    if original.pointer != seed.pointer {
                        return Err("selected transport original unit observation unavailable");
                    }
                    Ok(original)
                });
                let (weight, coastal) = match original {
                    Ok(original) => {
                        // The weight memo uses the original authenticated stat
                        // pointer. Shared stat failures are already memoized by
                        // the originals; this leaf also attempts only once.
                        let coastal = memo.capture(memory, base, original);
                        let weight = original.stat.and_then(|stat| {
                            #[cfg(test)]
                            {
                                self.probes += 1;
                            }
                            if let Some(value) = self.weights.get(&stat.pointer) {
                                return *value;
                            }
                            let value = at(stat.pointer, WEIGHT).and_then(|address| {
                                read(memory, address)
                                    .ok_or("selected transport original stat weight unreadable")
                            });
                            // Admission reserved one entry per division.
                            self.weights.insert(stat.pointer, value);
                            value
                        });
                        (weight, coastal)
                    }
                    Err(reason) => (Err(reason), Err(reason)),
                };
                let transfer = identity
                    .and_then(|_| {
                        seed.naval_transfer_membership
                            .ok_or("selected transport original transfer selector unavailable")
                    })
                    .and_then(|pointer| {
                        #[cfg(test)]
                        {
                            self.probes += 1;
                        }
                        if let Some(value) = self.transfers.get(&pointer) {
                            return *value;
                        }
                        let value = transfer(memory, base, Ok(pointer), true, |handle| {
                            registry.resolve(memory, handle)
                        });
                        // Admission reserved one entry per division.
                        self.transfers.insert(pointer, value);
                        value
                    });
                self.rows.push(Row {
                    unit: seed.division,
                    sample_hour: hour,
                    weight,
                    transfer,
                    coastal,
                });
            }
            self.required.members = memo.required_members;
            Ok(crate::walk::Outcome::Rows { len })
        })();
        self.outcome = result.unwrap_or_else(|reason| crate::walk::Outcome::Unavailable { reason });
        self.rows.sort_unstable_by_key(|row| row.unit);
        if self
            .rows
            .windows(2)
            .any(|pair| pair[0].unit == pair[1].unit)
        {
            self.outcome = crate::walk::Outcome::Unavailable {
                reason: "selected transport original unit identity repeated",
            };
        }
        self.transfers.clear();
        self.weights.clear();
    }
}

fn at(pointer: usize, offset: usize) -> Fact<usize> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("selected transport original pointer null or overflow")
}

#[cfg(test)]
#[path = "selected_transport_units_tests.rs"]
mod tests;
