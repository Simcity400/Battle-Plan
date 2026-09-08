//! CAP deployment registry resolver input, sampled inside the existing
//! registered division reader bracket.
//!
//! Native law is the retained registry writer closure for
//! `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, executable SHA256
//! `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
//! Cited instruction slices (verified retained D/I artifacts under
//! `20260729T004829Z-d75-pending-insert-allocator-seam-da4bc052/evidence`
//! for the factory and
//! `20260728T124939Z-xui16-engine-text-helper-abi-and-destruc-c407196c/evidence`
//! for growth, reread for this module):
//!
//! * `14220b090` `14220b141`/`14220b18a`/`14220b1e3` request three `0x20`-byte
//!   registry objects and publish them to `143438e50`, `143438e58` and the
//!   hundred cells `[143438e60, 143439180)` (`14220b1d5`/`14220b1dc`). Each
//!   fresh header is empty: `R+8 = &1430a5ca8`, `R+10`/`R+14`/`R+18` zero.
//!   No generation field exists in that header.
//! * `14220a4f0` `14220a76d..14220a7c5` fixes the grown shape: `ECX = k`,
//!   `DIL = k + 2` (overflow), `EBX = B = 1 << k`, `R+14 = B - 1` (mask),
//!   `R+18 = overflow`, request `24 * (B + overflow + 1)` with the native
//!   overflow-to-`SIZE_MAX` `CMOVC`, and `R+8` receives the allocation base.
//!   `24 * (B + overflow + 1) == 24 * (mask + overflow + 2)`.
//! * `14220a4f0` `14220a53a`/`14220a6dd` bound the exponent the factory can
//!   publish to `k = 3 ..= 30`; every other shape reaches the native
//!   diagnostic path at `14220a87d`.
//! * `14220a7c9..14220a7f4` zeroes the distance byte of buckets
//!   `[0, B + overflow)` and stores the terminal distance `0xff` at bucket
//!   `B + overflow`, i.e. index `mask + overflow + 1`.
//! * `14220a7ed`/`14220a7f8` compares old `S` against the shared static empty
//!   table `1430a5ca8` before freeing it, so that address is the factory
//!   static-empty shape rather than a heap allocation.
//!
//! The probe bit law itself (packed type-high key, wrapping 64-bit multiplier,
//! signed 32-bit bucket, wrapping 32-bit sentinel index, 24-byte forward step,
//! unsigned distance compare and the wrapping `CL` that runs `255 -> 0 -> 1`)
//! was reread from the verified D/I artifacts in
//! `20260717T124521Z-x15-oil-final-air-helper-leaves-b367eb8b/evidence`.
//! Root selection is `14220a3d0` in
//! `20260716T022603Z-ui12-base-icon-object-producers-d1-v1-4bf3ee84/evidence`.
//! The both-zero short circuit is `140bec560` at `140bec56b..140bec583`
//! in `20260716T023253Z-ui12-base-icon-callbacks-d1-v1-ab8edb5a/evidence`.
//!
//! What this module does NOT establish, per that same closure: the registry
//! header carries no generation, the key counter at `143439180` is not a
//! transaction sequence, and equal opening/closing endpoint bytes are not an
//! atomic snapshot. `14220a790..14220a82c` publishes a new mask before the new
//! `S`, so a stable-looking intermediate table is observable. This is a
//! current, original-hour bracketed observation only; lifetime, atomicity and
//! non-ABA reuse remain the parent separate admission question.

use super::*;
use country_roster_readset::Budget;
use std::alloc::{alloc, Layout};
use std::mem::size_of;

/// Raw deployment handle words on the division primary object.
const DIVISION_OFF_DEPLOYMENT_TYPE: usize = 0x32c;
const DIVISION_OFF_DEPLOYMENT_ID: usize = 0x330;

/// Three-tier root publication, `14220b090`.
const REGISTRY_ROOT_LOW_RVA: usize = 0x0343_8e60;
const REGISTRY_ROOT_MID_RVA: usize = 0x0343_8e58;
const REGISTRY_ROOT_HIGH_RVA: usize = 0x0343_8e50;
const REGISTRY_ROOT_LOW_COUNT: u32 = 100;
const REGISTRY_ROOT_MID_LIMIT: u32 = 0x1269;

/// Shared static empty table, `14220a7ed`.
const REGISTRY_STATIC_EMPTY_RVA: usize = 0x030a_5ca8;

const REGISTRY_BUCKET_STRIDE: usize = 24;
const REGISTRY_HASH_MULTIPLIER: u64 = 0x4963_1a45;
const REGISTRY_MIN_EXPONENT: u32 = 3;
const REGISTRY_MAX_EXPONENT: u32 = 30;

/// Every field this readset can reach. Refusals name the exact field so a
/// range or budget stop is never reported as a native lookup outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    PrimaryVtable,
    BodyVtable,
    DivisionHandle,
    DeploymentType,
    DeploymentId,
    RegistryRoot,
    RegistrySlots,
    RegistryMask,
    RegistryOverflow,
    ProbeDistance,
    ProbeId,
    ProbeType,
    MatchedValue,
}

/// The native terminal that ended the walk without resolving the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Miss {
    /// Sealed distance byte below the current `CL`.
    Distance,
    /// Full key match landed on the terminal bucket `mask + overflow + 1`.
    Sentinel,
}

/// Current, original-hour bracketed resolver outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Resolved {
    /// Both handle words zero. Native uninitialized reference; no registry read.
    ZeroPair,
    /// The selected root cell holds a sealed zero.
    RegistryNull,
    /// A native terminal was reached.
    Miss(Miss),
    /// Full key match on a non-sentinel bucket whose value qword is zero.
    MatchedNull,
    /// Full key match on a non-sentinel bucket whose value qword is nonzero.
    /// Presence only: the value is never dereferenced and never retained.
    ResolvedNonNull,
}

/// A precise refusal. Never a native miss.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// The enclosing division envelope did not authenticate.
    InvalidIdentity,
    /// The guarded reader declined a funded opening or closing read.
    Unreadable(Field),
    /// Closing bytes differed from the sealed opening bytes.
    Changed(Field),
    /// Checked address arithmetic left the representable range.
    AddressOverflow,
    /// The selected span is outside the admitted logical table extent or its
    /// base fails the heap-address gate. Distinct from a native miss; guarded
    /// copy/range failures are reported together as Unreadable.
    RangeRefused(Field),
    /// `S` is the static empty table but its reached distance byte does not
    /// give the native initial miss. The unproved static tail is not followed.
    UnsupportedStaticShape,
    /// The observed header is not a shape the retained factory/grow paths
    /// publish, so no logical allocation extent is admitted. Not a native miss.
    AllocationShapeUnproved,
    /// The shared verification ceiling refused before the read.
    VerificationBound,
    /// The shared retained ceiling refused before allocation.
    RetainedBound,
    /// A funded trace allocation failed before the source read.
    AllocationFailed,
}

/// The raw deployment handle, retained independently of the lookup outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DeploymentKey {
    pub native_type: u32,
    pub id: u32,
}

/// Opening-time raw header bits, kept for shape attribution only. These are
/// not sealed authority: a refused or changed result still reports what the
/// opening read observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RawHeader {
    pub mask: u32,
    pub overflow: u8,
}

/// PROVISIONAL local endpoint observations, pending the caller's original frame/campaign,
/// root and native registry custody verdicts. Equal endpoints do not establish
/// allocation lifetime or absence of ABA. A registry refusal preserves the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use = "provisional endpoints require original frame/campaign/root and native custody admission"]
pub(super) struct DeploymentObservation {
    pub key: Result<DeploymentKey, Refusal>,
    pub header: Option<RawHeader>,
    pub result: Result<Resolved, Refusal>,
}

#[cfg(test)]
#[path = "division_deployment_inputs_tests.rs"]
mod tests;

// ---------------------------------------------------------------------------
// Failure-retaining trace: every funded opening owns a funded closing, and
// every funded closing is attempted even after an earlier read or budget
// failure. Copied in shape from the sibling training readset so both charge
// the same shared pools the same way.
// ---------------------------------------------------------------------------

struct Witness {
    address: usize,
    width: usize,
    bytes: [u8; 8],
    field: Field,
    obtained: bool,
}
struct Block {
    rows: [Option<Witness>; 32],
    used: usize,
    next: Option<Box<Block>>,
}
const _: () = assert!(size_of::<Block>() <= 8192);

struct Trace {
    head: Option<Box<Block>>,
    // Must obey the global allocator's Layout contract, including null on
    // failure. The injected allocator is private and used by fault fixtures.
    allocate: unsafe fn(Layout) -> *mut u8,
}
impl Default for Trace {
    fn default() -> Self {
        Self {
            head: None,
            allocate: alloc,
        }
    }
}
impl Trace {
    /// Fund the opening and its closing together, then read. A failed opening
    /// keeps its closing funded and still records the witness.
    fn field<const N: usize>(
        &mut self,
        budget: &Budget,
        address: usize,
        field: Field,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<[u8; N], Refusal> {
        if N == 0 || N > 8 || address.checked_add(N).is_none() {
            return Err(Refusal::AddressOverflow);
        }
        if self.head.as_ref().is_none_or(|block| block.used == 32) {
            let layout = Layout::new::<Block>();
            budget
                .retain(layout.size())
                .map_err(|_| Refusal::RetainedBound)?;
            // Keep the existing tickets owned until the new allocation succeeds.
            // Box takes this exact initialized allocation and deallocates it with
            // the same global allocator and Layout, including padding/alignment.
            let mut block = unsafe {
                let pointer = (self.allocate)(layout).cast::<Block>();
                if pointer.is_null() {
                    return Err(Refusal::AllocationFailed);
                }
                pointer.write(Block {
                    rows: std::array::from_fn(|_| None),
                    used: 0,
                    next: None,
                });
                Box::from_raw(pointer)
            };
            block.next = self.head.take();
            self.head = Some(block);
        }
        budget
            .reserve_verification(N * 2)
            .map_err(|_| Refusal::VerificationBound)?;
        let mut bytes = [0; 8];
        let obtained = read(address, &mut bytes[..N]);
        let block = self.head.as_mut().unwrap();
        block.rows[block.used] = Some(Witness {
            address,
            width: N,
            bytes,
            field,
            obtained,
        });
        block.used += 1;
        if !obtained {
            return Err(Refusal::Unreadable(field));
        }
        Ok(bytes[..N].try_into().unwrap())
    }
    /// Re-read every funded original address and width. Never follows a
    /// replacement pointer; a changed seal outranks an unreadable one.
    fn close(&mut self, read: &mut impl FnMut(usize, &mut [u8]) -> bool) -> Result<(), Refusal> {
        let mut failure = None;
        while let Some(mut block) = self.head.take() {
            self.head = block.next.take();
            for row in block.rows.iter_mut().take(block.used).rev() {
                let witness = row.take().unwrap();
                let mut after = [0; 8];
                let obtained = read(witness.address, &mut after[..witness.width]);
                if !obtained || !witness.obtained {
                    failure.get_or_insert(Refusal::Unreadable(witness.field));
                } else if after[..witness.width] != witness.bytes[..witness.width] {
                    failure = Some(Refusal::Changed(witness.field));
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        while let Some(mut block) = self.head.take() {
            self.head = block.next.take();
        }
    }
}

fn at(base: usize, offset: usize) -> Result<usize, Refusal> {
    base.checked_add(offset).ok_or(Refusal::AddressOverflow)
}

// ---------------------------------------------------------------------------
// Native bit law. These functions keep the signed/wrapping arithmetic exactly
// as handed, independent of the extent-admission layer above them, so a
// supplied non-factory shape still decodes by native rules.
// ---------------------------------------------------------------------------

/// Packed type-high key times the wrapping 64-bit multiplier, folded to a
/// signed 32-bit bucket index by the observed mask.
fn probe_bucket(key: DeploymentKey, mask: u32) -> i32 {
    let hash = ((u64::from(key.native_type) << 32) | u64::from(key.id))
        .wrapping_mul(REGISTRY_HASH_MULTIPLIER);
    (((hash as u32) ^ ((hash >> 32) as u32)) & mask) as i32
}

/// Terminal bucket index, mirroring the native 32-bit add before sign extension.
fn sentinel_index(mask: u32, overflow: u8) -> i32 {
    mask.wrapping_add(u32::from(overflow) + 1) as i32
}

/// Sign-extended 24-byte forward address step with checked arithmetic.
fn signed_slot(slots: usize, index: i32) -> Result<usize, Refusal> {
    let delta = i64::from(index) * REGISTRY_BUCKET_STRIDE as i64;
    let delta = isize::try_from(delta).map_err(|_| Refusal::AddressOverflow)?;
    slots
        .checked_add_signed(delta)
        .ok_or(Refusal::AddressOverflow)
}

/// Native probe counter: starts at 1 and wraps `255 -> 0 -> 1`. A wrapped
/// `CL` of 0 does not terminate on a zero distance byte.
fn next_cl(cl: u8) -> u8 {
    cl.wrapping_add(1)
}

/// Three-tier root cell selection, `14220b090`.
fn root_slot(module: usize, native_type: u32) -> Result<usize, Refusal> {
    if native_type < REGISTRY_ROOT_LOW_COUNT {
        at(module, REGISTRY_ROOT_LOW_RVA + native_type as usize * 8)
    } else if native_type < REGISTRY_ROOT_MID_LIMIT {
        at(module, REGISTRY_ROOT_MID_RVA)
    } else {
        at(module, REGISTRY_ROOT_HIGH_RVA)
    }
}

/// Logical shape from the retained grow path. This is not allocation custody:
/// new mask/overflow can be visible while slots still names the old allocation.
#[derive(Clone, Copy)]
struct Table {
    slots: usize,
    mask: u32,
    overflow: u8,
    end: usize,
}
impl Table {
    /// Admit only the shapes `14220a4f0` can publish, then derive the checked
    /// logical extent `[S, S + 24 * (mask + overflow + 2))`. This names the
    /// writer's requested shape; it proves neither the currently live allocation
    /// nor that the native reader applies such a gate.
    fn admit(slots: usize, mask: u32, overflow: u8) -> Result<Self, Refusal> {
        let buckets = u64::from(mask) + 1;
        if !buckets.is_power_of_two() {
            return Err(Refusal::AllocationShapeUnproved);
        }
        let exponent = buckets.trailing_zeros();
        if !(REGISTRY_MIN_EXPONENT..=REGISTRY_MAX_EXPONENT).contains(&exponent)
            || u32::from(overflow) != exponent + 2
        {
            return Err(Refusal::AllocationShapeUnproved);
        }
        let cells = u64::from(mask) + u64::from(overflow) + 2;
        let extent = cells
            .checked_mul(REGISTRY_BUCKET_STRIDE as u64)
            .and_then(|extent| usize::try_from(extent).ok())
            .ok_or(Refusal::AddressOverflow)?;
        let end = slots.checked_add(extent).ok_or(Refusal::AddressOverflow)?;
        Ok(Self {
            slots,
            mask,
            overflow,
            end,
        })
    }
    /// Require the whole selected span inside the derived extent.
    fn holds(&self, address: usize, width: usize) -> bool {
        address >= self.slots
            && address
                .checked_add(width)
                .is_some_and(|end| end <= self.end)
    }
}

/// Authenticate the enclosing division envelope and return the primary object.
/// This is the same still-open identity relation the sibling readsets use.
fn authenticate(
    module: usize,
    body: usize,
    kind: u32,
    id: u32,
    budget: &Budget,
    trace: &mut Trace,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> Result<usize, Refusal> {
    let primary = body
        .checked_sub(DIVISION_WRAPPER_TO_BODY)
        .ok_or(Refusal::InvalidIdentity)?;
    if !is_map_control_heap_pointer(primary) {
        return Err(Refusal::InvalidIdentity);
    }
    let wrapper =
        u64::from_le_bytes(trace.field::<8>(budget, primary, Field::PrimaryVtable, read)?);
    let secondary = u64::from_le_bytes(trace.field::<8>(budget, body, Field::BodyVtable, read)?);
    let handle = trace.field::<8>(
        budget,
        at(body, DIVISION_OFF_ID_TYPE)?,
        Field::DivisionHandle,
        read,
    )?;
    if wrapper != at(module, DIVISION_WRAPPER_VTABLE_RVA)? as u64
        || secondary != at(module, DIVISION_VTABLE_RVA)? as u64
        || !is_compact_division_body_type(kind)
        || u32::from_le_bytes(handle[..4].try_into().unwrap()) != kind
        || u32::from_le_bytes(handle[4..].try_into().unwrap()) != id
    {
        return Err(Refusal::InvalidIdentity);
    }
    Ok(primary)
}

/// Read the raw deployment handle from the still-open primary object.
fn read_key(
    primary: usize,
    budget: &Budget,
    trace: &mut Trace,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> Result<DeploymentKey, Refusal> {
    let native_type = u32::from_le_bytes(trace.field::<4>(
        budget,
        at(primary, DIVISION_OFF_DEPLOYMENT_TYPE)?,
        Field::DeploymentType,
        read,
    )?);
    let id = u32::from_le_bytes(trace.field::<4>(
        budget,
        at(primary, DIVISION_OFF_DEPLOYMENT_ID)?,
        Field::DeploymentId,
        read,
    )?);
    Ok(DeploymentKey { native_type, id })
}

/// Select the root, open the header, admit the shape and run the native walk.
fn resolve(
    module: usize,
    key: DeploymentKey,
    budget: &Budget,
    trace: &mut Trace,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> (Option<RawHeader>, Result<Resolved, Refusal>) {
    if key.native_type == 0 && key.id == 0 {
        return (None, Ok(Resolved::ZeroPair));
    }
    let root = match root_slot(module, key.native_type) {
        Ok(root) => root,
        Err(cause) => return (None, Err(cause)),
    };
    let registry = match trace.field::<8>(budget, root, Field::RegistryRoot, read) {
        Ok(bytes) => u64::from_le_bytes(bytes),
        Err(cause) => return (None, Err(cause)),
    };
    if registry == 0 {
        return (None, Ok(Resolved::RegistryNull));
    }
    let Ok(registry) = usize::try_from(registry) else {
        return (None, Err(Refusal::AddressOverflow));
    };
    if !is_map_control_heap_pointer(registry) {
        return (None, Err(Refusal::RangeRefused(Field::RegistryRoot)));
    }
    // The 0x20-byte factory header. Only the three reached fields are opened;
    // the dead padding is never copied and never sealed.
    let header = (|| {
        let slots = u64::from_le_bytes(trace.field::<8>(
            budget,
            at(registry, 8)?,
            Field::RegistrySlots,
            read,
        )?);
        let slots = usize::try_from(slots).map_err(|_| Refusal::AddressOverflow)?;
        let mask = u32::from_le_bytes(trace.field::<4>(
            budget,
            at(registry, 0x14)?,
            Field::RegistryMask,
            read,
        )?);
        let overflow =
            trace.field::<1>(budget, at(registry, 0x18)?, Field::RegistryOverflow, read)?[0];
        Ok((slots, mask, overflow))
    })();
    let (slots, mask, overflow) = match header {
        Ok(header) => header,
        Err(cause) => return (None, Err(cause)),
    };
    let raw = Some(RawHeader { mask, overflow });
    let static_empty = match at(module, REGISTRY_STATIC_EMPTY_RVA) {
        Ok(address) => address,
        Err(cause) => return (raw, Err(cause)),
    };
    if slots == static_empty {
        return (raw, static_miss(slots, mask, overflow, budget, trace, read));
    }
    if !is_map_control_heap_pointer(slots) {
        return (raw, Err(Refusal::RangeRefused(Field::RegistrySlots)));
    }
    let table = match Table::admit(slots, mask, overflow) {
        Ok(table) => table,
        Err(cause) => return (raw, Err(cause)),
    };
    (raw, walk(table, key, budget, trace, read))
}

/// The static empty table publishes `mask = 0`, `overflow = 0` and a zero
/// distance byte at its only reached bucket. A sealed zero there is the native
/// initial miss; anything else refuses rather than following an unproved tail.
fn static_miss(
    slots: usize,
    mask: u32,
    overflow: u8,
    budget: &Budget,
    trace: &mut Trace,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> Result<Resolved, Refusal> {
    if mask != 0 || overflow != 0 {
        return Err(Refusal::AllocationShapeUnproved);
    }
    let distance = trace.field::<1>(budget, at(slots, 4)?, Field::ProbeDistance, read)?[0];
    if distance == 0 {
        Ok(Resolved::Miss(Miss::Distance))
    } else {
        Err(Refusal::UnsupportedStaticShape)
    }
}

/// The handed lookup walk. Distance gates the ID read, the ID gates the type
/// read, and only a non-sentinel full key match reaches the value qword. The
/// extent gate bounds the walk; it refuses by name and never reports a miss.
fn walk(
    table: Table,
    key: DeploymentKey,
    budget: &Budget,
    trace: &mut Trace,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> Result<Resolved, Refusal> {
    let mut slot = signed_slot(table.slots, probe_bucket(key, table.mask))?;
    let sentinel = signed_slot(table.slots, sentinel_index(table.mask, table.overflow))?;
    let mut cl: u8 = 1;
    loop {
        let distance =
            read_within::<1>(table, slot, 4, Field::ProbeDistance, budget, trace, read)?[0];
        if distance < cl {
            return Ok(Resolved::Miss(Miss::Distance));
        }
        let id = u32::from_le_bytes(read_within::<4>(
            table,
            slot,
            0xc,
            Field::ProbeId,
            budget,
            trace,
            read,
        )?);
        if id == key.id {
            let native_type = u32::from_le_bytes(read_within::<4>(
                table,
                slot,
                8,
                Field::ProbeType,
                budget,
                trace,
                read,
            )?);
            if native_type == key.native_type {
                if slot == sentinel {
                    return Ok(Resolved::Miss(Miss::Sentinel));
                }
                let value = u64::from_le_bytes(read_within::<8>(
                    table,
                    slot,
                    0x10,
                    Field::MatchedValue,
                    budget,
                    trace,
                    read,
                )?);
                // Presence only. The value is never dereferenced.
                return Ok(if value == 0 {
                    Resolved::MatchedNull
                } else {
                    Resolved::ResolvedNonNull
                });
            }
        }
        slot = at(slot, REGISTRY_BUCKET_STRIDE)?;
        cl = next_cl(cl);
    }
}

/// Require the whole selected span inside the logical extent before funding it.
fn read_within<const N: usize>(
    table: Table,
    slot: usize,
    offset: usize,
    field: Field,
    budget: &Budget,
    trace: &mut Trace,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> Result<[u8; N], Refusal> {
    let address = at(slot, offset)?;
    if !table.holds(address, N) {
        return Err(Refusal::RangeRefused(field));
    }
    trace.field::<N>(budget, address, field, read)
}

/// Sample local deployment endpoints inside the caller's original bracket.
/// The caller must supply the admitted module and shared frame budget, a
/// guarded callback whose transitive allocations are funded, and withdraw the
/// result on original identity/root/hour/campaign or native custody failure.
/// This helper creates no clock, campaign, allocation or mutation witness and
/// must not by itself authorize a portable measured resolver outcome.
pub(super) fn sample(
    module: usize,
    body: usize,
    kind: u32,
    id: u32,
    budget: &Budget,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> DeploymentObservation {
    let mut identity = Trace::default();
    let mut handle = Trace::default();
    let mut registry = Trace::default();

    let primary = authenticate(module, body, kind, id, budget, &mut identity, read);
    let key = primary.and_then(|primary| read_key(primary, budget, &mut handle, read));
    let (header, result) = match key {
        Ok(key) => resolve(module, key, budget, &mut registry, read),
        Err(cause) => (None, Err(cause)),
    };

    // Every funded close is attempted, in reverse funding order, after any
    // failure above. No refunds and no second budget.
    let registry_seal = registry.close(read);
    let handle_seal = handle.close(read);
    let identity_seal = identity.close(read);

    // The handle survives a registry-side refusal; the envelope poisons both.
    let sealed_handle = identity_seal.and(handle_seal);
    DeploymentObservation {
        key: sealed_handle.and(key),
        header,
        result: sealed_handle.and(registry_seal).and(result),
    }
}
