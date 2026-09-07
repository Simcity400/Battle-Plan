//! One scheduled D22 acquisition, sharing the country-roster frame quota.
use super::{country_roster_readset::Budget, is_map_control_heap_pointer};
use compact_contract::province_occupants::{
    self as wire, Cause, Census, Family, Province, Reference, Slot, SlotValue, Status, Vector,
};
use std::mem::size_of;

#[derive(Clone, Copy)]
enum Target {
    Clock,
    Root,
    Province(usize),
    Vector(usize, usize),
    Slot(usize, usize, usize),
    SlotReference(usize, usize, usize),
}
struct Witness {
    address: usize,
    bytes: [u8; 32],
    width: u8,
    obtained: bool,
    target: Target,
}
struct ReadSet<'a> {
    budget: &'a Budget,
    witnesses: Vec<Witness>,
}
impl<'a> ReadSet<'a> {
    fn read<const N: usize>(
        &mut self,
        address: usize,
        target: Target,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<[u8; N], Cause> {
        if N > 32 || N == 0 || address.checked_add(N).is_none() {
            return Err(Cause::BoundExceeded);
        }
        self.budget
            .retain(size_of::<Witness>())
            .map_err(|_| Cause::BoundExceeded)?;
        self.budget
            .reserve_verification(N * 2)
            .map_err(|_| Cause::BoundExceeded)?;
        self.witnesses.reserve_exact(1);
        let mut bytes = [0; 32];
        let obtained = read(address, &mut bytes[..N]);
        // Keep the reservation even when opening failed: finish still attempts
        // this original address. Never move the close into a fallible closure.
        self.witnesses.push(Witness {
            address,
            bytes,
            width: N as u8,
            obtained,
            target,
        });
        if !obtained {
            return Err(Cause::Unreadable);
        }
        Ok(bytes[..N].try_into().unwrap())
    }
    fn finish(self, out: &mut Census, read: &mut impl FnMut(usize, &mut [u8]) -> bool) {
        let mut bytes = [0; 32];
        for w in self.witnesses.into_iter().rev() {
            let width = w.width as usize;
            let obtained = read(w.address, &mut bytes[..width]);
            let cause = if !obtained || !w.obtained {
                Some(Cause::Unreadable)
            } else if bytes[..width] != w.bytes[..width] {
                Some(Cause::Changed)
            } else {
                None
            };
            if let Some(cause) = cause {
                // A stable prefix cannot create a class witness when opening
                // rejected the header. Preserve only the already admitted one.
                let admitted_class = match w.target {
                    Target::Slot(p, v, s) => out
                        .provinces
                        .get(p)
                        .and_then(|p| p.vectors.get(v))
                        .and_then(|v| v.slots.get(s))
                        .and_then(|s| s.value.class_witness()),
                    _ => None,
                };
                let class = if obtained && w.obtained && width == 32 && bytes[..24] == w.bytes[..24]
                {
                    admitted_class
                } else {
                    None
                };
                invalidate(out, w.target, cause);
                if let (Target::Slot(p, v, s), Some(class)) = (w.target, class) {
                    if let Some(slot) = out
                        .provinces
                        .get_mut(p)
                        .and_then(|p| p.vectors.get_mut(v))
                        .and_then(|v| v.slots.get_mut(s))
                    {
                        slot.value = SlotValue::Failed {
                            cause,
                            class_raw: Some(class),
                        };
                    }
                }
            }
        }
    }
}
fn at(p: usize, offset: usize) -> Result<usize, Cause> {
    p.checked_add(offset).ok_or(Cause::BoundExceeded)
}
fn u64b(v: [u8; 8]) -> usize {
    u64::from_le_bytes(v) as usize
}
fn word(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn partial(out: &mut Census, cause: Cause) {
    if out.status == Status::Measured {
        out.status = Status::Partial;
        out.cause = cause;
    }
}
fn first_cause(existing: &mut Cause, cause: Cause) {
    if matches!(*existing, Cause::None | Cause::DependencyUnavailable) {
        *existing = cause;
    }
}
fn partial_family(status: &mut Status, existing: &mut Cause, cause: Cause) {
    if *status != Status::Failed || *existing == Cause::DependencyUnavailable {
        *status = Status::Partial;
    }
    first_cause(existing, cause);
}
fn invalidate(out: &mut Census, target: Target, cause: Cause) {
    partial(out, cause);
    match target {
        Target::Clock => {
            out.status = Status::Failed;
            out.cause = if cause == Cause::Unreadable {
                Cause::ClockUnreadable
            } else {
                Cause::ClockChanged
            };
        }
        Target::Root => {
            out.status = Status::Failed;
            out.cause = cause;
        }
        Target::Province(p) => {
            if let Some(p) = out.provinces.get_mut(p) {
                p.status = Status::Failed;
                first_cause(&mut p.cause, cause);
                p.province_id = None;
                for v in &mut p.vectors {
                    v.status = Status::Failed;
                    first_cause(&mut v.cause, cause);
                    v.slots.clear();
                    v.covered_prefix = 0;
                }
            }
        }
        Target::Vector(p, v) => {
            if let Some(p) = out.provinces.get_mut(p) {
                partial_family(&mut p.status, &mut p.cause, cause);
                if let Some(v) = p.vectors.get_mut(v) {
                    v.status = Status::Failed;
                    first_cause(&mut v.cause, cause);
                    v.slots.clear();
                    v.covered_prefix = 0;
                }
            }
        }
        Target::Slot(p, v, s) | Target::SlotReference(p, v, s) => {
            if let Some(p) = out.provinces.get_mut(p) {
                partial_family(&mut p.status, &mut p.cause, cause);
                if let Some(v) = p.vectors.get_mut(v) {
                    partial_family(&mut v.status, &mut v.cause, cause);
                    if let Some(s) = v.slots.get_mut(s) {
                        let class_raw = if cause == Cause::IdentityConflict
                            || matches!(target, Target::SlotReference(..))
                        {
                            s.value.class_witness()
                        } else {
                            None
                        };
                        s.value = SlotValue::Failed { cause, class_raw };
                    }
                }
            }
        }
    }
}
fn empty_vector(family: Family) -> Vector {
    Vector {
        family,
        status: Status::Failed,
        cause: Cause::DependencyUnavailable,
        declared_count: None,
        covered_prefix: 0,
        slots: Vec::new(),
    }
}
fn shape(base: usize, vt: usize) -> Option<(usize, i32, bool)> {
    [
        (0x2943ad0, 0x2943cb0, 0),
        (0x294c480, 0x294c660, 1),
        (0x295b600, 0x295b7d8, 13),
    ]
    .into_iter()
    .find_map(|(a, b, c)| {
        if base.checked_add(a) == Some(vt) {
            Some((a, c, false))
        } else if base.checked_add(b) == Some(vt) {
            Some((a, c, true))
        } else {
            None
        }
    })
}
fn identity(
    base: usize,
    pointer: usize,
    target: Target,
    class_witness: &mut Option<i32>,
    set: &mut ReadSet<'_>,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> Result<(Reference, usize), Cause> {
    if !is_map_control_heap_pointer(pointer) {
        return Err(Cause::InvalidIdentity);
    }
    let first = set.witnesses.len();
    let vtable = set.read::<8>(pointer, target, read)?;
    let (primary, class, secondary) = shape(base, u64b(vtable)).ok_or(Cause::UnsupportedClass)?;
    let complete = if secondary {
        pointer.checked_sub(16).ok_or(Cause::InvalidIdentity)?
    } else {
        pointer
    };
    if !is_map_control_heap_pointer(complete) {
        return Err(Cause::InvalidIdentity);
    }
    let mut header = [0; 32];
    if secondary {
        header[16..24].copy_from_slice(&vtable);
        header[..16].copy_from_slice(&set.read::<16>(complete, target, read)?);
        if u64b(header[..8].try_into().unwrap()) != at(base, primary)? {
            return Err(Cause::InvalidIdentity);
        }
        // The two vtables and actual class word were obtained independently
        // of this last reference read. Their own original reads still close.
        *class_witness = Some(word(&header, 8) as i32);
        if *class_witness != Some(class) {
            return Err(Cause::ClassMismatch);
        }
        let reference_target = match target {
            Target::Slot(p, v, s) => Target::SlotReference(p, v, s),
            _ => target,
        };
        header[24..].copy_from_slice(&set.read::<8>(at(complete, 24)?, reference_target, read)?);
    } else {
        header[..8].copy_from_slice(&vtable);
        header[8..].copy_from_slice(&set.read::<24>(at(complete, 8)?, target, read)?);
    }
    // Successful normalization closes the single original complete header.
    // Opening failure leaves each already-funded piece in the set to close.
    set.witnesses[first] = Witness {
        address: complete,
        bytes: header,
        width: 32,
        obtained: true,
        target,
    };
    set.witnesses.truncate(first + 1);
    let second = match class {
        0 => 0x2943cb0,
        1 => 0x294c660,
        13 => 0x295b7d8,
        _ => unreachable!(),
    };
    if u64b(header[..8].try_into().unwrap()) != at(base, primary)?
        || u64b(header[16..24].try_into().unwrap()) != at(base, second)?
    {
        return Err(Cause::InvalidIdentity);
    }
    *class_witness = Some(word(&header, 8) as i32);
    if *class_witness != Some(class) {
        return Err(Cause::ClassMismatch);
    }
    let reference = Reference {
        class_raw: class,
        type_raw: word(&header, 24),
        id_raw: word(&header, 28),
    };
    if reference.type_raw == 0 && reference.id_raw == 0 {
        return Err(Cause::UninitializedReference);
    }
    Ok((reference, complete))
}
#[derive(Clone, Copy)]
struct Seen {
    reference: Reference,
    complete: usize,
    conflicted: bool,
}
fn identity_key(reference: Reference) -> (u32, u32) {
    (reference.type_raw, reference.id_raw)
}
fn invalidate_reference(out: &mut Census, reference: Reference) {
    partial(out, Cause::IdentityConflict);
    for p in &mut out.provinces {
        for v in &mut p.vectors {
            for slot in &mut v.slots {
                if let SlotValue::Measured(observed) = slot.value {
                    if identity_key(observed) != identity_key(reference) {
                        continue;
                    }
                    slot.value = SlotValue::Failed {
                        cause: Cause::IdentityConflict,
                        class_raw: Some(observed.class_raw),
                    };
                    partial_family(&mut v.status, &mut v.cause, Cause::IdentityConflict);
                    partial_family(&mut p.status, &mut p.cause, Cause::IdentityConflict);
                }
            }
        }
    }
}
/// The opening ordinal comes from the existing first-reader clock bracket.
/// The readset owns fresh clock/root/body observations. The row-producing
/// topology supplies expected identities only; it never supplies observed bytes.
pub(super) fn capture(
    base: usize,
    gs: usize,
    hour: i32,
    ordinal: u64,
    topology: &super::CompactMapControlTopology,
    budget: &Budget,
    mut read: impl FnMut(usize, &mut [u8]) -> bool,
) -> Census {
    let mut out = Census::failed(hour, ordinal, Cause::BoundExceeded);
    // Charge all three leaf destinations before allocation: Captured::Data,
    // the worker MAPC payload, and the final H4CR frame. Fixed MAPC envelopes
    // and the record header are reserved here too. Existing control-row costs
    // remain with their owner. No charge is refunded when a later read fails.
    if budget
        .retain(wire::HEADER_LEN * 3 + 40 * 2 + usize::from(super::COMPACT_RECORD_HEADER_LEN))
        .is_err()
    {
        return out;
    }
    let mut set = ReadSet {
        budget,
        witnesses: Vec::new(),
    };
    let mut seen: Vec<Seen> = Vec::new();
    let mut total_slots = 0usize;
    let result = (|| -> Result<(), Cause> {
        if !is_map_control_heap_pointer(gs) {
            return Err(Cause::InvalidHeader);
        }
        let singleton = at(
            base,
            compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
        )?;
        if u64b(set.read::<8>(singleton, Target::Root, &mut read)?) != gs {
            return Err(Cause::Changed);
        }
        if i32::from_le_bytes(set.read::<4>(
            at(gs, super::GS_CURRENT_DATE_OFFSET)?,
            Target::Clock,
            &mut read,
        )?) != hour
        {
            return Err(Cause::ClockChanged);
        }
        let root = u64b(set.read::<8>(
            at(gs, super::CGAMESTATE_CPROVINCE_ARRAY)?,
            Target::Root,
            &mut read,
        )?);
        let count = u32::from_le_bytes(set.read::<4>(
            at(gs, super::CGAMESTATE_CPROVINCE_COUNT)?,
            Target::Root,
            &mut read,
        )?);
        if !(2..=wire::MAX_PROVINCES as u32).contains(&count) || !is_map_control_heap_pointer(root)
        {
            return Err(Cause::InvalidHeader);
        }
        if root != topology.province_array || count != topology.province_count {
            return Err(Cause::Changed);
        }
        out.root_slot_count = Some(count);
        out.status = Status::Measured;
        out.cause = Cause::None;
        budget
            .retain(count as usize * size_of::<usize>())
            .map_err(|_| Cause::BoundExceeded)?;
        let mut pointers = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            pointers.push(u64b(set.read::<8>(
                at(root, i * 8)?,
                Target::Root,
                &mut read,
            )?));
        }
        // These local pointers are the topology that supplied this acquisition
        // of D22 control rows. Fresh cells must agree even on a cache hit.
        // Reject before following a replacement object with the same numeric ID.
        for (pi, pointer) in pointers.iter().copied().enumerate() {
            if topology.province_ptr(pi as u32).unwrap_or(0) != pointer {
                return Err(Cause::Changed);
            }
        }
        for (pi, pointer) in pointers.into_iter().enumerate() {
            budget
                .retain(
                    size_of::<Province>()
                        + 4 * size_of::<Vector>()
                        + 3 * (wire::PROVINCE_LEN + 4 * wire::VECTOR_LEN),
                )
                .map_err(|_| Cause::BoundExceeded)?;
            out.provinces.reserve_exact(1);
            let mut province = Province {
                physical_ordinal: pi as u32,
                province_id: None,
                status: Status::Failed,
                cause: Cause::DependencyUnavailable,
                vectors: Vec::with_capacity(4),
            };
            if pointer == 0 {
                province.status = Status::Vacant;
                province.cause = Cause::None;
                out.provinces.push(province);
                out.root_covered_prefix += 1;
                continue;
            }
            for family in Family::ALL {
                province.vectors.push(empty_vector(family));
            }
            out.provinces.push(province);
            out.root_covered_prefix += 1;
            let target = Target::Province(pi);
            let acquired = (|| -> Result<(), Cause> {
                if !is_map_control_heap_pointer(pointer) {
                    return Err(Cause::InvalidIdentity);
                }
                if u64b(set.read::<8>(at(root, pi * 8)?, target, &mut read)?) != pointer {
                    return Err(Cause::Changed);
                }
                if u64b(set.read::<8>(pointer, target, &mut read)?)
                    != at(base, super::CPROVINCE_VTABLE_RVA)?
                {
                    return Err(Cause::InvalidIdentity);
                }
                let id = u32::from_le_bytes(set.read::<4>(
                    at(pointer, super::CPROVINCE_OFF_ID)?,
                    target,
                    &mut read,
                )?);
                // This is the existing CProvince table identity check, not an
                // inferred state/template/renderer ordinal.
                if id == 0 || id as usize != pi {
                    return Err(Cause::InvalidIdentity);
                }
                out.provinces[pi].province_id = Some(id);
                out.provinces[pi].status = Status::Measured;
                out.provinces[pi].cause = Cause::None;
                for (vi, offset) in [0xe0, 0x110, 0xf8, 0x128].into_iter().enumerate() {
                    let result = (|| -> Result<(), Cause> {
                        let header = set.read::<16>(
                            at(pointer, offset)?,
                            Target::Vector(pi, vi),
                            &mut read,
                        )?;
                        let array = u64b(header[..8].try_into().unwrap());
                        let capacity = word(&header, 8) as i32;
                        let declared = word(&header, 12) as i32;
                        if declared < 0 || capacity < declared {
                            return Err(Cause::InvalidHeader);
                        }
                        out.provinces[pi].vectors[vi].declared_count = Some(declared);
                        if declared as usize > wire::MAX_SLOTS {
                            return Err(Cause::BoundExceeded);
                        }
                        if declared != 0 && !is_map_control_heap_pointer(array) {
                            return Err(Cause::InvalidHeader);
                        }
                        out.provinces[pi].vectors[vi].status = Status::Measured;
                        out.provinces[pi].vectors[vi].cause = Cause::None;
                        for si in 0..declared as usize {
                            if total_slots == wire::MAX_SLOTS {
                                return Err(Cause::BoundExceeded);
                            }
                            budget
                                .retain(size_of::<Slot>() + 3 * wire::SLOT_LEN + size_of::<Seen>())
                                .map_err(|_| Cause::BoundExceeded)?;
                            out.provinces[pi].vectors[vi].slots.reserve_exact(1);
                            seen.reserve_exact(1);
                            let target = Target::Slot(pi, vi, si);
                            let mut class_witness = None;
                            let observed = (|| -> Result<SlotValue, Cause> {
                                let pointer =
                                    u64b(set.read::<8>(at(array, si * 8)?, target, &mut read)?);
                                if pointer == 0 {
                                    return Ok(SlotValue::Null);
                                }
                                let (reference, complete) = identity(
                                    base,
                                    pointer,
                                    target,
                                    &mut class_witness,
                                    &mut set,
                                    &mut read,
                                )?;
                                if Family::ALL[vi]
                                    .class()
                                    .is_some_and(|class| class != reference.class_raw)
                                {
                                    return Err(Cause::ClassMismatch);
                                }
                                match seen.binary_search_by_key(&identity_key(reference), |s| {
                                    identity_key(s.reference)
                                }) {
                                    Ok(index)
                                        if seen[index].conflicted
                                            || seen[index].complete != complete
                                            || seen[index].reference.class_raw
                                                != reference.class_raw =>
                                    {
                                        seen[index].conflicted = true;
                                        invalidate_reference(&mut out, reference);
                                        return Err(Cause::IdentityConflict);
                                    }
                                    Ok(_) => {}
                                    Err(index) => seen.insert(
                                        index,
                                        Seen {
                                            reference,
                                            complete,
                                            conflicted: false,
                                        },
                                    ),
                                }
                                Ok(SlotValue::Measured(reference))
                            })();
                            let value = match observed {
                                Ok(v) => v,
                                Err(cause) => {
                                    partial(&mut out, cause);
                                    let p = &mut out.provinces[pi];
                                    partial_family(&mut p.status, &mut p.cause, cause);
                                    let v = &mut p.vectors[vi];
                                    partial_family(&mut v.status, &mut v.cause, cause);
                                    SlotValue::Failed {
                                        cause,
                                        class_raw: class_witness,
                                    }
                                }
                            };
                            out.provinces[pi].vectors[vi].slots.push(Slot {
                                physical_ordinal: si as u32,
                                value,
                            });
                            out.provinces[pi].vectors[vi].covered_prefix += 1;
                            total_slots += 1;
                            if matches!(
                                value,
                                SlotValue::Failed {
                                    cause: Cause::BoundExceeded,
                                    ..
                                }
                            ) {
                                return Err(Cause::BoundExceeded);
                            }
                        }
                        Ok(())
                    })();
                    if let Err(cause) = result {
                        partial(&mut out, cause);
                        let p = &mut out.provinces[pi];
                        partial_family(&mut p.status, &mut p.cause, cause);
                        let v = &mut p.vectors[vi];
                        partial_family(&mut v.status, &mut v.cause, cause);
                    }
                }
                Ok(())
            })();
            if let Err(cause) = acquired {
                invalidate(&mut out, target, cause);
                if cause == Cause::BoundExceeded {
                    return Err(cause);
                }
            }
        }
        Ok(())
    })();
    if let Err(cause) = result {
        if out.provinces.is_empty() {
            out.status = Status::Failed;
            out.cause = cause;
        } else {
            partial(&mut out, cause);
        }
    }
    set.finish(&mut out, &mut read);
    out
}

/// A stack refusal can always be carried when the shared retained quota is
/// already exhausted; it does not allocate an invented empty observation.
const REFUSAL_PUBLICATION_BYTES: usize =
    2 * (40 + wire::HEADER_LEN) + super::COMPACT_RECORD_HEADER_LEN as usize;
pub(super) enum Captured {
    Failed([u8; wire::HEADER_LEN]),
    Unfunded([u8; wire::HEADER_LEN]),
    Data(Vec<u8>),
}
impl Captured {
    pub(super) fn failed(hour: i32, ordinal: u64, cause: Cause) -> Self {
        let mut bytes = [0; wire::HEADER_LEN];
        bytes[..8].copy_from_slice(&[b'P', b'O', b'C', b'1', 1, 0, 48, 0]);
        bytes[8..12].copy_from_slice(&hour.to_le_bytes());
        bytes[16..24].copy_from_slice(&ordinal.to_le_bytes());
        bytes[36] = Status::Failed as u8;
        bytes[37] = cause as u8;
        Self::Unfunded(bytes)
    }
    fn funded_failed(hour: i32, ordinal: u64, cause: Cause) -> Self {
        let Self::Unfunded(bytes) = Self::failed(hour, ordinal, cause) else {
            unreachable!()
        };
        Self::Failed(bytes)
    }
    pub(super) fn failure(hour: i32, ordinal: u64, cause: Cause, budget: &Budget) -> Self {
        if budget.retain(REFUSAL_PUBLICATION_BYTES).is_ok() {
            Self::funded_failed(hour, ordinal, cause)
        } else {
            Self::failed(hour, ordinal, cause)
        }
    }
    pub(super) fn publication_funded(&self) -> bool {
        !matches!(self, Self::Unfunded(_))
    }
    pub(super) fn bytes(&self) -> &[u8] {
        match self {
            Self::Failed(b) | Self::Unfunded(b) => b,
            Self::Data(b) => b,
        }
    }
    pub(super) fn invalidates_topology(&self) -> bool {
        matches!(
            Cause::from_raw(self.bytes()[37]),
            Ok(Cause::Changed | Cause::InvalidHeader | Cause::InvalidIdentity | Cause::Unreadable)
        )
    }
    pub(super) fn is_measured(&self) -> bool {
        self.bytes()[36] == Status::Measured as u8
    }
}
pub(super) fn capture_encoded(
    base: usize,
    gs: usize,
    hour: i32,
    ordinal: u64,
    topology: &super::CompactMapControlTopology,
    budget: &Budget,
    read: impl FnMut(usize, &mut [u8]) -> bool,
) -> Captured {
    // Retain a fixed refusal publication before any acquisition consumes the
    // remaining quota. If even this cannot fit, the owning tail emits D22 GAP
    // through its existing control path, without encoding a MAPC payload.
    if budget.retain(REFUSAL_PUBLICATION_BYTES).is_err() {
        return Captured::failed(hour, ordinal, Cause::BoundExceeded);
    }
    let census = capture(base, gs, hour, ordinal, topology, budget, read);
    if census.status == Status::Failed {
        return Captured::funded_failed(hour, ordinal, census.cause);
    }
    match wire::encode(&census) {
        Ok(bytes) => Captured::Data(bytes),
        Err(_) => Captured::funded_failed(hour, ordinal, Cause::InvalidHeader),
    }
}

#[cfg(test)]
pub(super) mod tests;
