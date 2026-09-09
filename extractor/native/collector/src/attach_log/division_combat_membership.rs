//! Fixed-primary-side linked country operands for the existing first-combat acquisition.
//!
//! Source: FIRST_COMBAT_SIDE_COUNTRY_MEMBERSHIP.md and superseding
//! BORDERWAR_SIDE_IDENTITY_CLOSED.md in Temp/cap3-first-combat-membership-design-
//! 01bf74b481f5495798dcce46f0fc4eb4. The latter pins a729/870ebf constructor
//! 1413d6840 -> 1413d6610 -> 141295330 -> 1413cb4f0 and preservation tails
//! 1413cc0d0/1413cebe0: BorderWar C+28 points to a complete side with vptr
//! 1429a5818 and backlink S+18=C. Land instead requires 1429913c0.
//!
//! Mounted through division_presentation_inputs::current_motion_inputs_with_membership
//! by sample_division_movement_with_training. FirstCombatInput is a value-only
//! view of the open CombatRead. The adapter keeps division, vector/first-entry,
//! combat, root and hour witnesses open through membership acquisition; its
//! enclosing callback drains the preflight and OpenedFrame authority closes.
//! Every funded original-address close runs even after refusal.
//! An old closed sample, equal id/hour, or `|| Ok(())` is not authority.
//!
//! The side recipe is 64+24N bytes. The fresh parent envelope is 176 bytes,
//! giving 240+24N; no-first-combat is parent-only 112 bytes. Motion, paired
//! country and enclosing authority witnesses are additional funded reads.
//! The adapter samples signed primary+1e0 inside this acquisition and the
//! mounted caller publishes it through TrainingScalars.country_operand.
//! sample_without_country supplies the other training scalars.
//! Owner+1d8 is independent; neither operand is resolved here.
//! No country filtering, sorting, deduplication, lookup or match shortcut.
use super::{country_roster_readset::Budget, HEAP_HI, HEAP_LO};
use compact_contract::division_routes::{CombatClass, CombatIdentity};
use std::alloc::{alloc, Layout};
use std::mem::size_of;
use std::ptr;

const TRACE_CAPACITY: usize = 32;
const NODE_BLOCK_CAPACITY: usize = 256;
pub(super) const SIDE_VERIFICATION_BYTES: usize = 64;
pub(super) const NODE_VERIFICATION_BYTES: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    FrameRoot,
    Hour,
    DivisionIdentity,
    FirstCombatVector,
    FirstCombatPointer,
    CombatIdentity,
    SidePointer,
    SideClass,
    SideBacklink,
    Head,
    Country,
    Next,
    OrdinaryRoute,
    AlternateRoute,
    RouteHead,
    Numerator,
    Transfer,
    Target,
    Retreat,
    DivisionCountryOperand,
    AttachmentIdentity,
    CampaignIdentity,
    ProducerBinding,
    Strength,
    Organisation,
    RetainedVisualVariant,
    DivisionOwner,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Admission,
    Open,
    Close,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cause {
    NotObserved,
    Unreadable,
    Changed,
    InvalidIdentity,
    NullSide,
    Cycle,
    VerificationBudgetExceeded,
    RetainedBudgetExceeded,
    AllocationFailed,
    AddressOverflow,
    InvalidAddress,
    RootChanged,
    HourChanged,
    HourUnreadable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Refusal {
    pub cause: Cause,
    pub field: Field,
    pub stage: Stage,
    pub ordinal: Option<usize>,
    pub original_ordinal: Option<usize>,
}
impl Refusal {
    pub(super) fn at(cause: Cause, field: Field, stage: Stage, ordinal: Option<usize>) -> Self {
        Self {
            cause,
            field,
            stage,
            ordinal,
            original_ordinal: None,
        }
    }
}
/// Values borrowed from the actual open CombatRead; no authentication capability.
#[derive(Clone, Copy, Debug)]
pub(super) struct FirstCombatInput {
    pub primary: usize,
    pub identity: CombatIdentity,
}
#[derive(Debug)]
pub(super) enum FirstCombatCountries {
    NoFirstCombat,
    Present {
        combat: CombatIdentity,
        countries: Result<OrderedCountries, Refusal>,
    },
    Unavailable(Refusal),
}

#[derive(Default)]
pub(super) struct Allocation {
    #[cfg(test)]
    attempts: usize,
    #[cfg(test)]
    fail_at: Option<usize>,
}
fn charged_box<T>(
    budget: &Budget,
    allocation: &mut Allocation,
    value: T,
    field: Field,
    ordinal: Option<usize>,
) -> Result<Box<T>, Refusal> {
    let pointer = charged_allocation(Layout::new::<T>(), budget, allocation, field, ordinal)?;
    // Ownership of exactly this initialized allocation transfers to Box.
    unsafe {
        let pointer = pointer.cast::<T>();
        pointer.write(value);
        Ok(Box::from_raw(pointer))
    }
}
fn charged_allocation(
    layout: Layout,
    budget: &Budget,
    allocation: &mut Allocation,
    field: Field,
    ordinal: Option<usize>,
) -> Result<*mut u8, Refusal> {
    budget.retain(layout.size()).map_err(|_| {
        Refusal::at(
            Cause::RetainedBudgetExceeded,
            field,
            Stage::Admission,
            ordinal,
        )
    })?;
    #[cfg(test)]
    {
        allocation.attempts += 1;
        if allocation.fail_at == Some(allocation.attempts) {
            return Err(Refusal::at(
                Cause::AllocationFailed,
                field,
                Stage::Admission,
                ordinal,
            ));
        }
    }
    #[cfg(not(test))]
    let _ = allocation;
    // Callers supply nonzero layouts. Failed allocation remains charged.
    let pointer = unsafe { alloc(layout) };
    if pointer.is_null() {
        Err(Refusal::at(
            Cause::AllocationFailed,
            field,
            Stage::Admission,
            ordinal,
        ))
    } else {
        Ok(pointer)
    }
}
/// Exact-layout route output, so no allocator-hidden Vec capacity is unfunded.
/// Length remains zero until the caller appends initialized u32 values.
pub(super) fn charged_u32_buffer(
    count: usize,
    budget: &Budget,
    allocation: &mut Allocation,
    field: Field,
) -> Result<Vec<u32>, Refusal> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let layout = Layout::array::<u32>(count)
        .map_err(|_| Refusal::at(Cause::AddressOverflow, field, Stage::Admission, None))?;
    let pointer = charged_allocation(layout, budget, allocation, field, None)?;
    // Vec deallocates with the same global allocator, u32 alignment and capacity.
    Ok(unsafe { Vec::from_raw_parts(pointer.cast::<u32>(), 0, count) })
}

fn address(
    base: usize,
    offset: usize,
    width: usize,
    field: Field,
    ordinal: Option<usize>,
) -> Result<usize, Refusal> {
    let at = base
        .checked_add(offset)
        .ok_or_else(|| Refusal::at(Cause::AddressOverflow, field, Stage::Admission, ordinal))?;
    let end = at
        .checked_add(width)
        .ok_or_else(|| Refusal::at(Cause::AddressOverflow, field, Stage::Admission, ordinal))?;
    if !(HEAP_LO..HEAP_HI).contains(&base) || base % 8 != 0 || end > HEAP_HI {
        return Err(Refusal::at(
            Cause::InvalidAddress,
            field,
            Stage::Admission,
            ordinal,
        ));
    }
    Ok(at)
}
struct Witness {
    address: usize,
    width: usize,
    bytes: [u8; 8],
    obtained: bool,
    field: Field,
    ordinal: Option<usize>,
}
struct TraceBlock {
    rows: [Option<Witness>; TRACE_CAPACITY],
    used: usize,
    next: Option<Box<TraceBlock>>,
}
#[derive(Default)]
pub(super) struct Trace {
    head: Option<Box<TraceBlock>>,
}
impl Trace {
    fn prepare(
        &mut self,
        count: usize,
        budget: &Budget,
        allocation: &mut Allocation,
        field: Field,
        ordinal: Option<usize>,
    ) -> Result<(), Refusal> {
        assert!(count <= TRACE_CAPACITY);
        if self
            .head
            .as_ref()
            .is_some_and(|b| b.used + count <= TRACE_CAPACITY)
        {
            return Ok(());
        }
        let mut block = charged_box(
            budget,
            allocation,
            TraceBlock {
                rows: std::array::from_fn(|_| None),
                used: 0,
                next: None,
            },
            field,
            ordinal,
        )?;
        block.next = self.head.take();
        self.head = Some(block);
        Ok(())
    }
    fn reserve(&mut self, at: usize, width: usize, field: Field, ordinal: Option<usize>) -> usize {
        let b = self.head.as_mut().unwrap();
        let index = b.used;
        b.rows[index] = Some(Witness {
            address: at,
            width,
            bytes: [0; 8],
            obtained: false,
            field,
            ordinal,
        });
        b.used += 1;
        index
    }
    fn open(
        &mut self,
        index: usize,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<[u8; 8], Refusal> {
        let w = self.head.as_mut().unwrap().rows[index].as_mut().unwrap();
        w.obtained = read(w.address, &mut w.bytes[..w.width]);
        if w.obtained {
            Ok(w.bytes)
        } else {
            Err(Refusal::at(
                Cause::Unreadable,
                w.field,
                Stage::Open,
                w.ordinal,
            ))
        }
    }
    /// Record the obligation before the attempted read, including failed opens.
    /// Address custody/range checks belong to the decoder (module slots are not heap).
    pub(super) fn word(
        &mut self,
        at: usize,
        width: usize,
        field: Field,
        budget: &Budget,
        allocation: &mut Allocation,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<[u8; 8], Refusal> {
        if !(1..=8).contains(&width) || at.checked_add(width).is_none() {
            return Err(Refusal::at(
                Cause::AddressOverflow,
                field,
                Stage::Admission,
                None,
            ));
        }
        self.prepare(1, budget, allocation, field, None)?;
        budget.reserve_verification(width * 2).map_err(|_| {
            Refusal::at(
                Cause::VerificationBudgetExceeded,
                field,
                Stage::Admission,
                None,
            )
        })?;
        let index = self.reserve(at, width, field, None);
        self.open(index, read)
    }
    fn pointer(
        &mut self,
        base: usize,
        offset: usize,
        field: Field,
        budget: &Budget,
        allocation: &mut Allocation,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<usize, Refusal> {
        let at = address(base, offset, 8, field, None)?;
        self.prepare(1, budget, allocation, field, None)?;
        budget.reserve_verification(16).map_err(|_| {
            Refusal::at(
                Cause::VerificationBudgetExceeded,
                field,
                Stage::Admission,
                None,
            )
        })?;
        let index = self.reserve(at, 8, field, None);
        let bytes = self.open(index, read)?;
        usize::try_from(u64::from_le_bytes(bytes))
            .map_err(|_| Refusal::at(Cause::InvalidAddress, field, Stage::Open, None))
    }
    pub(super) fn close(
        &mut self,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<(), Refusal> {
        let mut failure = None;
        while let Some(mut block) = self.head.take() {
            self.head = block.next.take();
            for row in block.rows[..block.used].iter_mut().rev() {
                let w = row.take().unwrap();
                let mut after = [0; 8];
                let obtained = read(w.address, &mut after[..w.width]);
                let cause = if !obtained {
                    Some(Cause::Unreadable)
                } else if w.obtained && after[..w.width] != w.bytes[..w.width] {
                    Some(Cause::Changed)
                } else {
                    None
                };
                if let Some(cause) = cause {
                    if failure.is_none() || cause == Cause::Changed {
                        failure = Some(Refusal::at(cause, w.field, Stage::Close, w.ordinal));
                    }
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        while let Some(mut b) = self.head.take() {
            self.head = b.next.take();
        }
    }
}
#[derive(Clone, Copy)]
struct NodeRecord {
    address: usize,
    next: usize,
    country: i32,
    ordinal: usize,
    left: *mut NodeRecord,
    right: *mut NodeRecord,
    height: usize,
}
impl NodeRecord {
    const EMPTY: Self = Self {
        address: 0,
        next: 0,
        country: 0,
        ordinal: 0,
        left: ptr::null_mut(),
        right: ptr::null_mut(),
        height: 1,
    };
}
struct NodeBlock {
    rows: [NodeRecord; NODE_BLOCK_CAPACITY],
    used: usize,
    next: Option<Box<NodeBlock>>,
}
const _: () = assert!(size_of::<NodeBlock>() < 192 * 1024);
const _: () = assert!(size_of::<TraceBlock>() < 192 * 1024);
/// Owns charged blocks. Only ordered country words are exposed, never addresses.
#[derive(Default)]
pub(super) struct OrderedCountries {
    head: Option<Box<NodeBlock>>,
    tail: *mut NodeBlock,
    root: *mut NodeRecord,
    len: usize,
}
// All links point into exclusively owned stable Box allocations. No reference
// to a record escapes. Moving ownership between existing sample workers is safe.
unsafe impl Send for OrderedCountries {}
impl std::fmt::Debug for OrderedCountries {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}
impl OrderedCountries {
    pub(super) fn len(&self) -> usize {
        self.len
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = i32> + '_ {
        std::iter::successors(self.head.as_deref(), |b| b.next.as_deref())
            .flat_map(|b| b.rows[..b.used].iter().map(|r| r.country))
    }
    /// Consume a sealed complete census into a funded exact-layout portable
    /// word buffer. This is an additional allocation; freeing scratch refunds
    /// nothing. Callers must fund subsequent baseline and encoding copies.
    pub(super) fn try_into_raw_words(self, budget: &Budget) -> Result<Vec<i32>, Refusal> {
        if self.len == 0 {
            return Ok(Vec::new());
        }
        let layout = Layout::array::<i32>(self.len).map_err(|_| {
            Refusal::at(
                Cause::AddressOverflow,
                Field::Country,
                Stage::Admission,
                None,
            )
        })?;
        let pointer = charged_allocation(
            layout,
            budget,
            &mut Allocation::default(),
            Field::Country,
            None,
        )?;
        // Same global allocator/layout as Vec, initialized only by bounded push.
        let mut words = unsafe { Vec::from_raw_parts(pointer.cast::<i32>(), 0, self.len) };
        for raw in self.iter() {
            words.push(raw);
        }
        Ok(words)
    }
    fn prepare(&mut self, budget: &Budget, allocation: &mut Allocation) -> Result<(), Refusal> {
        if !self.tail.is_null() && unsafe { (*self.tail).used < NODE_BLOCK_CAPACITY } {
            return Ok(());
        }
        let mut block = charged_box(
            budget,
            allocation,
            NodeBlock {
                rows: [NodeRecord::EMPTY; NODE_BLOCK_CAPACITY],
                used: 0,
                next: None,
            },
            Field::Country,
            Some(self.len),
        )?;
        let new_tail = &mut *block as *mut NodeBlock;
        if self.tail.is_null() {
            self.head = Some(block);
        } else {
            unsafe {
                (*self.tail).next = Some(block);
            }
        }
        self.tail = new_tail;
        Ok(())
    }
    fn find(&self, address: usize) -> Option<usize> {
        let mut current = self.root;
        // Private links are initialized only by insert/rotations, never native data.
        unsafe {
            while !current.is_null() {
                if address == (*current).address {
                    return Some((*current).ordinal);
                }
                current = if address < (*current).address {
                    (*current).left
                } else {
                    (*current).right
                };
            }
        }
        None
    }
    fn insert(&mut self, address: usize) -> *mut NodeRecord {
        // prepare() guaranteed capacity; stable records are never moved/deleted.
        unsafe {
            // Do not form a mutable reference to the whole block: previously
            // inserted rows have live raw AVL links into that same allocation.
            let used = (*self.tail).used;
            let node = ptr::addr_of_mut!((*self.tail).rows)
                .cast::<NodeRecord>()
                .add(used);
            node.write(NodeRecord {
                address,
                ordinal: self.len,
                ..NodeRecord::EMPTY
            });
            (*self.tail).used = used + 1;
            self.len += 1;
            self.root = avl_insert(self.root, node);
            node
        }
    }
}
impl Drop for OrderedCountries {
    fn drop(&mut self) {
        while let Some(mut b) = self.head.take() {
            self.head = b.next.take();
        }
    }
}
// Intrusive AVL, keyed by complete node address. Its recursion is AVL height,
// not native chain length; duplicate native country values are never tree keys.
fn height(node: *mut NodeRecord) -> usize {
    if node.is_null() {
        0
    } else {
        unsafe { (*node).height }
    }
}
fn update(node: *mut NodeRecord) {
    unsafe {
        (*node).height = 1 + height((*node).left).max(height((*node).right));
    }
}
fn rotate_left(node: *mut NodeRecord) -> *mut NodeRecord {
    unsafe {
        let right = (*node).right;
        (*node).right = (*right).left;
        (*right).left = node;
        update(node);
        update(right);
        right
    }
}
fn rotate_right(node: *mut NodeRecord) -> *mut NodeRecord {
    unsafe {
        let left = (*node).left;
        (*node).left = (*left).right;
        (*left).right = node;
        update(node);
        update(left);
        left
    }
}
fn avl_insert(root: *mut NodeRecord, node: *mut NodeRecord) -> *mut NodeRecord {
    if root.is_null() {
        return node;
    }
    unsafe {
        if (*node).address < (*root).address {
            (*root).left = avl_insert((*root).left, node);
        } else {
            (*root).right = avl_insert((*root).right, node);
        }
        update(root);
        if height((*root).left) > height((*root).right) + 1 {
            if (*node).address > (*(*root).left).address {
                (*root).left = rotate_left((*root).left);
            }
            return rotate_right(root);
        }
        if height((*root).right) > height((*root).left) + 1 {
            if (*node).address < (*(*root).right).address {
                (*root).right = rotate_right((*root).right);
            }
            return rotate_left(root);
        }
        root
    }
}
fn side_countries(
    input: FirstCombatInput,
    module: usize,
    budget: &Budget,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    allocation: &mut Allocation,
) -> Result<OrderedCountries, Refusal> {
    let mut outer = Trace::default();
    let mut nodes = Trace::default();
    let mut countries = OrderedCountries::default();
    let opening = (|| {
        let key = input.identity;
        let side_rva = match key.class {
            CombatClass::Land => 0x29913c0,
            CombatClass::BorderWar => 0x29a5818,
        };
        let expected = module.checked_add(side_rva).ok_or_else(|| {
            Refusal::at(
                Cause::AddressOverflow,
                Field::SideClass,
                Stage::Admission,
                None,
            )
        })?;
        let side = outer.pointer(
            input.primary,
            0x28,
            Field::SidePointer,
            budget,
            allocation,
            read,
        )?;
        if side == 0 {
            return Err(Refusal::at(
                Cause::NullSide,
                Field::SidePointer,
                Stage::Open,
                None,
            ));
        }
        if outer.pointer(side, 0, Field::SideClass, budget, allocation, read)? != expected {
            return Err(Refusal::at(
                Cause::InvalidIdentity,
                Field::SideClass,
                Stage::Open,
                None,
            ));
        }
        if outer.pointer(side, 0x18, Field::SideBacklink, budget, allocation, read)?
            != input.primary
        {
            return Err(Refusal::at(
                Cause::InvalidIdentity,
                Field::SideBacklink,
                Stage::Open,
                None,
            ));
        }
        let mut node = outer.pointer(side, 0x38, Field::Head, budget, allocation, read)?;
        while node != 0 {
            let ordinal = countries.len;
            let country_at = address(node, 0, 4, Field::Country, Some(ordinal))?;
            let next_at = address(node, 16, 8, Field::Next, Some(ordinal))?;
            if let Some(original) = countries.find(node) {
                return Err(Refusal {
                    cause: Cause::Cycle,
                    field: Field::Next,
                    stage: Stage::Admission,
                    ordinal: Some(ordinal),
                    original_ordinal: Some(original),
                });
            }
            countries.prepare(budget, allocation)?;
            nodes.prepare(2, budget, allocation, Field::Country, Some(ordinal))?;
            budget
                .reserve_verification(NODE_VERIFICATION_BYTES)
                .map_err(|_| {
                    Refusal::at(
                        Cause::VerificationBudgetExceeded,
                        Field::Country,
                        Stage::Admission,
                        Some(ordinal),
                    )
                })?;
            let record = countries.insert(node);
            // Both funded obligations are recorded BEFORE either attempted read.
            let a = nodes.reserve(country_at, 4, Field::Country, Some(ordinal));
            let b = nodes.reserve(next_at, 8, Field::Next, Some(ordinal));
            let country = nodes.open(a, read);
            let next = nodes.open(b, read);
            let country = i32::from_le_bytes(country?[..4].try_into().unwrap());
            let next = usize::try_from(u64::from_le_bytes(next?)).map_err(|_| {
                Refusal::at(
                    Cause::InvalidAddress,
                    Field::Next,
                    Stage::Open,
                    Some(ordinal),
                )
            })?;
            unsafe {
                (*record).country = country;
                (*record).next = next;
            }
            node = next;
        }
        Ok(())
    })();
    // Never follow closing next/head/side pointers. All close attempts precede
    // choosing the refusal, including after allocation/shared-budget failures.
    let node_close = nodes.close(read);
    let side_close = outer.close(read);
    side_close.and(node_close).and(opening)?;
    Ok(countries)
}
/// `first` comes from the still-open current CombatRead; `close_parent` drains
/// original combat/first-entry/vector/division/frame closes from inner to outer
/// as required by the parent trace. This function does not create that trace.
/// Every normal return calls the callback, including unknown/empty first combat.
/// A parent failure invalidates all descendants; side-only failure retains the
/// independently sealed complete CombatIdentity. No callback may publish early.
pub(super) fn with_membership<R: FnMut(usize, &mut [u8]) -> bool>(
    module: usize,
    first: Result<Option<FirstCombatInput>, Refusal>,
    budget: &Budget,
    read: &mut R,
    close_parent: impl FnOnce(&mut R) -> Result<(), Refusal>,
) -> FirstCombatCountries {
    with_allocation(
        module,
        first,
        budget,
        read,
        close_parent,
        &mut Allocation::default(),
    )
}
fn with_allocation<R: FnMut(usize, &mut [u8]) -> bool>(
    module: usize,
    first: Result<Option<FirstCombatInput>, Refusal>,
    budget: &Budget,
    read: &mut R,
    close_parent: impl FnOnce(&mut R) -> Result<(), Refusal>,
    allocation: &mut Allocation,
) -> FirstCombatCountries {
    let candidate = match first {
        Ok(None) => FirstCombatCountries::NoFirstCombat,
        Err(reason) => FirstCombatCountries::Unavailable(reason),
        Ok(Some(input)) => {
            let key = input.identity;
            let invalid_key = key.id == 0
                || key.id > i32::MAX as u32
                || (key.class == CombatClass::Land && key.kind != 62);
            let primary = address(input.primary, 0x28, 8, Field::CombatIdentity, None);
            if invalid_key {
                FirstCombatCountries::Unavailable(Refusal::at(
                    Cause::InvalidIdentity,
                    Field::CombatIdentity,
                    Stage::Admission,
                    None,
                ))
            } else if let Err(reason) = primary {
                FirstCombatCountries::Unavailable(reason)
            } else {
                FirstCombatCountries::Present {
                    combat: key,
                    countries: side_countries(input, module, budget, read, allocation),
                }
            }
        }
    };
    match close_parent(read) {
        Ok(()) => candidate,
        Err(reason) => FirstCombatCountries::Unavailable(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const MODULE: usize = 0x140000000;
    const GS: usize = HEAP_LO + 0x10000;
    const DIVISION: usize = HEAP_LO + 0x20000;
    const VECTOR: usize = HEAP_LO + 0x30000;
    const COMBAT: usize = HEAP_LO + 0x40000;
    const SIDE: usize = HEAP_LO + 0x50000;
    const NODE: usize = HEAP_LO + 0x60000;
    const RETAINED: usize = 16 * 1024 * 1024;
    const VERIFICATION: usize = 10 * 1024 * 1024;

    struct Word {
        at: usize,
        bytes: Vec<u8>,
        field: Field,
    }
    enum Effect {
        Unreadable,
        Replace(Vec<u8>),
    }
    struct Fault {
        at: usize,
        visit: usize,
        effect: Effect,
    }
    struct Fixture {
        input: FirstCombatInput,
        empty_vector: bool,
        bytes: BTreeMap<usize, Vec<u8>>,
        faults: Vec<Fault>,
        visits: BTreeMap<usize, usize>,
        events: Vec<(usize, usize)>,
    }
    impl Fixture {
        fn new(class: CombatClass, countries: &[i32]) -> Self {
            let input = FirstCombatInput {
                primary: COMBAT,
                identity: CombatIdentity {
                    class,
                    kind: if class == CombatClass::Land {
                        62
                    } else {
                        0x1234
                    },
                    id: 79,
                },
            };
            let mut f = Self {
                input,
                empty_vector: false,
                bytes: BTreeMap::new(),
                faults: Vec::new(),
                visits: BTreeMap::new(),
                events: Vec::new(),
            };
            f.reset_parent();
            f.put64(COMBAT + 0x28, SIDE as u64);
            f.put64(
                SIDE,
                (MODULE
                    + if class == CombatClass::Land {
                        0x29913c0
                    } else {
                        0x29a5818
                    }) as u64,
            );
            f.put64(SIDE + 0x18, COMBAT as u64);
            f.chain(countries, &(0..countries.len()).collect::<Vec<_>>());
            f
        }
        // Test-only minimal envelope, with original addresses and actual read
        // attempts. The production adapter lives in division_presentation_inputs.
        fn parent_words(&self) -> Vec<Word> {
            let body = DIVISION + 0x10;
            let key = self.input.identity;
            let (primary_vptr, body_vptr) = match key.class {
                CombatClass::Land => (0x29914d8, 0x29915b8),
                CombatClass::BorderWar => (0x29a56e0, 0x29a57c0),
            };
            let mut words = Vec::new();
            let mut add = |at, value: u64, width, field| {
                words.push(Word {
                    at,
                    bytes: value.to_le_bytes()[..width].to_vec(),
                    field,
                });
            };
            add(MODULE + 0x3316e90, GS as u64, 8, Field::FrameRoot);
            add(GS + 0x468, 100, 4, Field::Hour);
            add(
                DIVISION,
                (MODULE + 0x2943ad0) as u64,
                8,
                Field::DivisionIdentity,
            );
            add(
                body,
                (MODULE + 0x2943cb0) as u64,
                8,
                Field::DivisionIdentity,
            );
            add(body + 8, 52, 4, Field::DivisionIdentity);
            add(body + 12, 7, 4, Field::DivisionIdentity);
            add(body + 0x18, 19, 4, Field::DivisionIdentity);
            add(body + 0x198, VECTOR as u64, 8, Field::FirstCombatVector);
            add(body + 0x1a0, 1, 4, Field::FirstCombatVector);
            add(
                body + 0x1a4,
                u64::from(!self.empty_vector),
                4,
                Field::FirstCombatVector,
            );
            if !self.empty_vector {
                add(
                    VECTOR,
                    self.input.primary as u64,
                    8,
                    Field::FirstCombatPointer,
                );
                add(
                    COMBAT,
                    (MODULE + primary_vptr) as u64,
                    8,
                    Field::CombatIdentity,
                );
                add(
                    COMBAT + 0x10,
                    (MODULE + body_vptr) as u64,
                    8,
                    Field::CombatIdentity,
                );
                add(COMBAT + 0x18, key.kind as u64, 4, Field::CombatIdentity);
                add(COMBAT + 0x1c, key.id as u64, 4, Field::CombatIdentity);
            }
            words
        }
        fn reset_parent(&mut self) {
            for w in self.parent_words() {
                self.bytes.insert(w.at, w.bytes);
            }
        }
        fn put64(&mut self, at: usize, value: u64) {
            self.bytes.insert(at, value.to_le_bytes().to_vec());
        }
        fn chain(&mut self, countries: &[i32], order: &[usize]) {
            assert_eq!(countries.len(), order.len());
            self.put64(SIDE + 0x38, order.first().map_or(0, |i| node(*i) as u64));
            for (ordinal, &index) in order.iter().enumerate() {
                self.bytes
                    .insert(node(index), countries[ordinal].to_le_bytes().to_vec());
                self.put64(
                    node(index) + 16,
                    order.get(ordinal + 1).map_or(0, |i| node(*i) as u64),
                );
            }
        }
        fn fail(&mut self, at: usize, visit: usize) {
            self.faults.push(Fault {
                at,
                visit,
                effect: Effect::Unreadable,
            });
        }
        fn change(&mut self, at: usize, visit: usize, bytes: Vec<u8>) {
            self.faults.push(Fault {
                at,
                visit,
                effect: Effect::Replace(bytes),
            });
        }
        fn read(&mut self, at: usize, out: &mut [u8]) -> bool {
            let count = self.visits.entry(at).or_default();
            *count += 1;
            let visit = *count;
            self.events.push((at, out.len()));
            for fault in &self.faults {
                if fault.at == at && fault.visit == visit {
                    match &fault.effect {
                        Effect::Unreadable => {
                            out.fill(0xa5);
                            return false;
                        }
                        Effect::Replace(bytes) => {
                            self.bytes.insert(at, bytes.clone());
                        }
                    }
                }
            }
            match self.bytes.get(&at) {
                Some(bytes) if bytes.len() == out.len() => {
                    out.copy_from_slice(bytes);
                    true
                }
                _ => false,
            }
        }
        fn sample(&mut self, budget: &Budget) -> FirstCombatCountries {
            self.sample_alloc(budget, &mut Allocation::default())
        }
        fn sample_alloc(
            &mut self,
            budget: &Budget,
            allocation: &mut Allocation,
        ) -> FirstCombatCountries {
            let mut parent = Trace::default();
            let first = (|| {
                for w in self.parent_words() {
                    parent.prepare(1, budget, allocation, w.field, None)?;
                    budget
                        .reserve_verification(w.bytes.len() * 2)
                        .map_err(|_| {
                            Refusal::at(
                                Cause::VerificationBudgetExceeded,
                                w.field,
                                Stage::Admission,
                                None,
                            )
                        })?;
                    let index = parent.reserve(w.at, w.bytes.len(), w.field, None);
                    let bytes = parent.open(index, &mut |at, out| self.read(at, out))?;
                    if &bytes[..w.bytes.len()] != w.bytes.as_slice() {
                        return Err(Refusal::at(
                            Cause::InvalidIdentity,
                            w.field,
                            Stage::Open,
                            None,
                        ));
                    }
                }
                Ok((!self.empty_vector).then_some(self.input))
            })();
            with_allocation(
                MODULE,
                first,
                budget,
                &mut |at, out| self.read(at, out),
                |read| parent.close(read),
                allocation,
            )
        }
        fn assert_all_closed(&self) {
            for (at, count) in &self.visits {
                assert_eq!(
                    *count, 2,
                    "original address {at:x} did not receive both attempts"
                );
            }
        }
        fn attempted_bytes(&self) -> usize {
            self.events.iter().map(|(_, n)| n).sum()
        }
    }
    fn node(index: usize) -> usize {
        NODE + index * 0x20
    }
    fn measured(result: &FirstCombatCountries) -> &OrderedCountries {
        match result {
            FirstCombatCountries::Present {
                countries: Ok(value),
                ..
            } => value,
            other => panic!("expected complete census, got {other:?}"),
        }
    }
    fn side_failure(result: &FirstCombatCountries) -> Refusal {
        match result {
            FirstCombatCountries::Present {
                countries: Err(reason),
                ..
            } => *reason,
            other => {
                panic!("expected independently sealed combat with side refusal, got {other:?}")
            }
        }
    }
    fn parent_failure(result: &FirstCombatCountries) -> Refusal {
        match result {
            FirstCombatCountries::Unavailable(reason) => *reason,
            other => panic!("expected outer refusal, got {other:?}"),
        }
    }
    fn retained_success(n: usize) -> usize {
        // One parent block and one side block; node witnesses are separate.
        (2 + (2 * n).div_ceil(TRACE_CAPACITY)) * size_of::<TraceBlock>()
            + n.div_ceil(NODE_BLOCK_CAPACITY) * size_of::<NodeBlock>()
    }

    #[test]
    fn ordered_signed_duplicates_and_exact_fresh_recipe_for_both_classes() {
        let words = [i32::MIN, 0, -1, i32::MAX, 0, -1];
        for class in [CombatClass::Land, CombatClass::BorderWar] {
            let mut f = Fixture::new(class, &words);
            let budget = Budget::test_limits(240 + 24 * words.len(), retained_success(words.len()));
            let result = f.sample(&budget);
            assert_eq!(measured(&result).iter().collect::<Vec<_>>(), words);
            assert_eq!(measured(&result).len(), words.len());
            match result {
                FirstCombatCountries::Present { combat, .. } => {
                    assert_eq!(combat, f.input.identity)
                }
                _ => unreachable!(),
            }
            assert_eq!(budget.verification_bytes(), 240 + 24 * words.len());
            assert_eq!(budget.retained_bytes(), retained_success(words.len()));
            assert_eq!(f.attempted_bytes(), budget.verification_bytes());
            assert!(!f.visits.contains_key(&(DIVISION + 0x1d8)));
            assert!(!f.visits.contains_key(&(DIVISION + 0x1e0)));
            assert!(!f.visits.contains_key(&(SIDE + 0xb8)));
            f.assert_all_closed();
        }
    }

    #[test]
    fn empty_vector_null_side_and_empty_head_are_distinct_and_sealed() {
        let mut empty = Fixture::new(CombatClass::Land, &[]);
        empty.empty_vector = true;
        empty.reset_parent();
        let budget = Budget::test_limits(112, RETAINED);
        assert!(matches!(
            empty.sample(&budget),
            FirstCombatCountries::NoFirstCombat
        ));
        assert_eq!(empty.attempted_bytes(), 112);
        assert_eq!(budget.verification_bytes(), 112);
        assert!(!empty.visits.contains_key(&VECTOR));
        empty.assert_all_closed();

        let mut null_side = Fixture::new(CombatClass::Land, &[]);
        null_side.put64(COMBAT + 0x28, 0);
        let budget = Budget::test_limits(192, RETAINED);
        assert_eq!(
            side_failure(&null_side.sample(&budget)).cause,
            Cause::NullSide
        );
        assert_eq!(null_side.attempted_bytes(), 192);
        null_side.assert_all_closed();

        let mut no_nodes = Fixture::new(CombatClass::Land, &[]);
        let budget = Budget::test_limits(240, retained_success(0));
        let result = no_nodes.sample(&budget);
        assert_eq!(measured(&result).len(), 0);
        assert_eq!(budget.retained_bytes(), retained_success(0));
        assert_eq!(no_nodes.attempted_bytes(), 240);
        no_nodes.assert_all_closed();
    }

    #[test]
    fn correlated_side_class_and_exact_original_backlink_are_required() {
        for class in [CombatClass::Land, CombatClass::BorderWar] {
            for (at, wrong, field) in [
                (
                    SIDE,
                    (MODULE
                        + if class == CombatClass::Land {
                            0x29a5818
                        } else {
                            0x29913c0
                        }) as u64,
                    Field::SideClass,
                ),
                (SIDE + 0x18, (COMBAT + 0x1000) as u64, Field::SideBacklink),
            ] {
                let mut f = Fixture::new(class, &[1]);
                f.put64(at, wrong);
                let refusal = side_failure(&f.sample(&Budget::default()));
                assert_eq!(
                    (refusal.cause, refusal.field),
                    (Cause::InvalidIdentity, field)
                );
                assert!(!f.visits.contains_key(&node(0)));
                f.assert_all_closed();
            }
        }
    }

    #[test]
    fn malformed_combat_context_cannot_be_present_or_empty() {
        for (class, kind, id) in [
            (CombatClass::Land, 61, 79),
            (CombatClass::Land, 62, 0),
            (CombatClass::BorderWar, 9, u32::MAX),
        ] {
            let mut f = Fixture::new(class, &[1]);
            f.input.identity.kind = kind;
            f.input.identity.id = id;
            f.reset_parent();
            assert_eq!(
                parent_failure(&f.sample(&Budget::default())).cause,
                Cause::InvalidIdentity
            );
            assert!(!f.visits.contains_key(&(COMBAT + 0x28)));
            f.assert_all_closed();
        }
        let mut f = Fixture::new(CombatClass::Land, &[1]);
        f.put64(VECTOR, 0); // Positive count, not NoFirstCombat.
        assert_eq!(
            parent_failure(&f.sample(&Budget::default())).field,
            Field::FirstCombatPointer
        );
        f.assert_all_closed();
    }

    #[test]
    fn every_opening_failure_still_attempts_funded_closes_and_no_prefix_escapes() {
        for (at, field) in [
            (COMBAT + 0x28, Field::SidePointer),
            (SIDE, Field::SideClass),
            (SIDE + 0x18, Field::SideBacklink),
            (SIDE + 0x38, Field::Head),
            (node(1), Field::Country),
            (node(1) + 16, Field::Next),
        ] {
            let mut f = Fixture::new(CombatClass::Land, &[44, 44, 0]);
            f.fail(at, 1);
            let budget = Budget::default();
            let refusal = side_failure(&f.sample(&budget));
            assert_eq!(
                (refusal.cause, refusal.field, refusal.stage),
                (Cause::Unreadable, field, Stage::Open)
            );
            if at >= NODE {
                assert_eq!(f.visits.get(&node(1)), Some(&2));
                assert_eq!(f.visits.get(&(node(1) + 16)), Some(&2));
                assert!(!f.visits.contains_key(&node(2)));
            }
            assert_eq!(f.attempted_bytes(), budget.verification_bytes());
            f.assert_all_closed();
        }
    }

    #[test]
    fn closing_failures_and_mutations_never_follow_replacement_pointers() {
        for at in [
            COMBAT + 0x28,
            SIDE,
            SIDE + 0x18,
            SIDE + 0x38,
            node(0),
            node(0) + 16,
        ] {
            for unreadable in [false, true] {
                let mut f = Fixture::new(CombatClass::BorderWar, &[0, -1]);
                let width = f.bytes[&at].len();
                if unreadable {
                    f.fail(at, 2);
                } else {
                    let mut replacement = f.bytes[&at].clone();
                    replacement[0] ^= 0x10;
                    f.change(at, 2, replacement);
                }
                let budget = Budget::default();
                let refusal = side_failure(&f.sample(&budget));
                assert_eq!(
                    refusal.cause,
                    if unreadable {
                        Cause::Unreadable
                    } else {
                        Cause::Changed
                    }
                );
                assert_eq!(refusal.stage, Stage::Close);
                assert_eq!(f.visits.len(), 15 + 4 + 4);
                assert_eq!(
                    f.events
                        .iter()
                        .filter(|(a, n)| *a == at && *n == width)
                        .count(),
                    2
                );
                assert_eq!(f.attempted_bytes(), 240 + 48);
                f.assert_all_closed();
            }
        }
    }

    #[test]
    fn parent_mutation_overrides_even_side_refusal_and_drains_outward() {
        let template = Fixture::new(CombatClass::Land, &[1]);
        for word in template.parent_words() {
            let mut f = Fixture::new(CombatClass::Land, &[1]);
            f.fail(node(0), 1);
            let mut replacement = word.bytes.clone();
            replacement[0] ^= 1;
            f.change(word.at, 2, replacement);
            let refusal = parent_failure(&f.sample(&Budget::default()));
            assert_eq!(
                (refusal.cause, refusal.field, refusal.stage),
                (Cause::Changed, word.field, Stage::Close)
            );
            let parent_close = f
                .events
                .iter()
                .rposition(|(at, _)| *at == COMBAT + 0x1c)
                .unwrap();
            let side_close = f
                .events
                .iter()
                .rposition(|(at, _)| *at == COMBAT + 0x28)
                .unwrap();
            assert!(parent_close > side_close);
            assert_eq!(f.events.last().unwrap().0, MODULE + 0x3316e90);
            f.assert_all_closed();
        }
        let mut empty = Fixture::new(CombatClass::Land, &[]);
        empty.empty_vector = true;
        empty.reset_parent();
        empty.fail(GS + 0x468, 2);
        assert_eq!(
            parent_failure(&empty.sample(&Budget::default())).field,
            Field::Hour
        );
        empty.assert_all_closed();
    }

    #[test]
    fn cycles_report_original_and_repeat_ordinals_without_rereading_repeated_node() {
        for (n, original) in [(1, 0), (4, 1), (257, 16)] {
            let mut f = Fixture::new(CombatClass::Land, &vec![7; n]);
            f.put64(node(n - 1) + 16, node(original) as u64);
            let budget = Budget::default();
            let refusal = side_failure(&f.sample(&budget));
            assert_eq!(
                (refusal.cause, refusal.ordinal, refusal.original_ordinal),
                (Cause::Cycle, Some(n), Some(original))
            );
            assert_eq!(f.attempted_bytes(), 240 + 24 * n);
            assert_eq!(budget.verification_bytes(), f.attempted_bytes());
            f.assert_all_closed();
        }
    }

    fn assert_tree(root: *mut NodeRecord, lower: usize, upper: usize) -> (usize, usize) {
        if root.is_null() {
            return (0, 0);
        }
        unsafe {
            let row = &*root;
            assert!(lower < row.address && row.address < upper);
            let (left_height, left_count) = assert_tree(row.left, lower, row.address);
            let (right_height, right_count) = assert_tree(row.right, row.address, upper);
            assert!(left_height.abs_diff(right_height) <= 1);
            assert_eq!(row.height, 1 + left_height.max(right_height));
            (row.height, 1 + left_count + right_count)
        }
    }

    #[test]
    fn multiple_blocks_preserve_arrival_order_and_balanced_full_address_index() {
        let n = 1025;
        let countries: Vec<_> = (0..n).map(|i| i as i32 - 512).collect();
        for order in [
            (0..n).collect::<Vec<_>>(),
            (0..n).rev().collect(),
            (0..n).step_by(2).chain((1..n).step_by(2)).collect(),
        ] {
            let mut f = Fixture::new(CombatClass::Land, &[]);
            f.chain(&countries, &order);
            let budget = Budget::test_limits(240 + 24 * n, retained_success(n));
            let result = f.sample(&budget);
            let output = measured(&result);
            assert_eq!(output.iter().collect::<Vec<_>>(), countries);
            assert_eq!(assert_tree(output.root, 0, usize::MAX).1, n);
            for (ordinal, &index) in order.iter().enumerate() {
                assert_eq!(output.find(node(index)), Some(ordinal));
            }
            assert_eq!(budget.retained_bytes(), retained_success(n));
            assert_eq!(budget.verification_bytes(), f.attempted_bytes());
            f.assert_all_closed();
        }
    }

    #[test]
    fn verification_exhaustion_is_not_termination_and_does_not_cancel_closes() {
        for n in [1, 17, 257] {
            let mut f = Fixture::new(CombatClass::Land, &vec![0; n]);
            let limit = 240 + 24 * n - 1;
            let budget = Budget::test_limits(limit, RETAINED);
            let refusal = side_failure(&f.sample(&budget));
            assert_eq!(
                (refusal.cause, refusal.ordinal),
                (Cause::VerificationBudgetExceeded, Some(n - 1))
            );
            assert!(!f.visits.contains_key(&node(n - 1)));
            assert_eq!(f.attempted_bytes(), 240 + 24 * (n - 1));
            assert_eq!(budget.verification_bytes(), limit + 1);
            f.assert_all_closed();
        }
    }

    #[test]
    fn every_allocation_failure_drains_existing_reads_including_growth() {
        let n = 257;
        let mut complete = Fixture::new(CombatClass::Land, &vec![0; n]);
        let mut allocation = Allocation::default();
        assert_eq!(
            measured(&complete.sample_alloc(&Budget::default(), &mut allocation)).len(),
            n
        );
        // Parent, side, two record blocks and 17 node witness blocks.
        assert_eq!(allocation.attempts, 21);
        for fail_at in 1..=allocation.attempts {
            let mut f = Fixture::new(CombatClass::Land, &vec![0; n]);
            let budget = Budget::default();
            let mut allocation = Allocation {
                attempts: 0,
                fail_at: Some(fail_at),
            };
            let result = f.sample_alloc(&budget, &mut allocation);
            let refusal = if fail_at == 1 {
                parent_failure(&result)
            } else {
                side_failure(&result)
            };
            assert_eq!(refusal.cause, Cause::AllocationFailed);
            assert_eq!(allocation.attempts, fail_at);
            assert!(budget.retained_bytes() > 0); // Failed allocation stays charged.
            assert_eq!(budget.verification_bytes(), f.attempted_bytes());
            f.assert_all_closed();
        }
    }

    #[test]
    fn retained_refusal_precedes_allocator_and_leaves_prior_closes_funded() {
        for limit in [
            0,
            size_of::<TraceBlock>(),
            2 * size_of::<TraceBlock>(),
            retained_success(16),
            retained_success(256),
        ] {
            let mut f = Fixture::new(CombatClass::Land, &vec![3; 257]);
            let budget = Budget::test_limits(VERIFICATION, limit);
            let mut allocation = Allocation::default();
            let result = f.sample_alloc(&budget, &mut allocation);
            let refusal = if limit == 0 {
                parent_failure(&result)
            } else {
                side_failure(&result)
            };
            assert_eq!(refusal.cause, Cause::RetainedBudgetExceeded);
            assert_eq!(budget.retained_bytes(), limit + 1);
            if limit == 0 {
                assert_eq!(allocation.attempts, 0);
            }
            if limit == retained_success(256) {
                assert_eq!(refusal.ordinal, Some(256));
                assert!(!f.visits.contains_key(&node(256)));
            }
            assert_eq!(budget.verification_bytes(), f.attempted_bytes());
            f.assert_all_closed();
        }
    }

    #[test]
    fn invalid_native_addresses_refuse_without_reading_them() {
        for invalid in [1usize, HEAP_HI, NODE + 1, usize::MAX] {
            let mut f = Fixture::new(CombatClass::Land, &[0]);
            f.put64(SIDE + 0x38, invalid as u64);
            let refusal = side_failure(&f.sample(&Budget::default()));
            assert!(matches!(
                refusal.cause,
                Cause::InvalidAddress | Cause::AddressOverflow
            ));
            assert!(!f.visits.contains_key(&invalid));
            assert_eq!(f.attempted_bytes(), 240);
            f.assert_all_closed();
        }
    }

    #[test]
    fn shared_quota_poison_during_open_does_not_cancel_any_funded_close() {
        let budget = Budget::test_limits(88, RETAINED);
        let mut f = Fixture::new(CombatClass::Land, &[1]);
        let mut read = |at, out: &mut [u8]| {
            let obtained = f.read(at, out);
            if at == node(0) {
                // Same atomic budget, as another existing worker would charge.
                assert!(budget.reserve_verification(1).is_err());
            }
            obtained
        };
        let mut parent_closed = false;
        let result = with_membership(
            MODULE,
            Ok(Some(FirstCombatInput {
                primary: COMBAT,
                identity: CombatIdentity {
                    class: CombatClass::Land,
                    kind: 62,
                    id: 79,
                },
            })),
            &budget,
            &mut read,
            |_| {
                parent_closed = true;
                Ok(())
            },
        );
        // This tests incremental closure only. The no-op parent is NOT a
        // production bracket or a qualification of the full 240-byte recipe.
        assert_eq!(measured(&result).iter().collect::<Vec<_>>(), [1]);
        assert!(parent_closed);
        assert_eq!(budget.verification_bytes(), 89);
        assert_eq!(f.attempted_bytes(), 88);
        f.assert_all_closed();
    }

    #[test]
    fn failed_open_then_failed_close_still_attempts_every_other_close() {
        let mut f = Fixture::new(CombatClass::Land, &[1, 2]);
        f.fail(node(1), 1);
        f.fail(node(1) + 16, 1);
        f.fail(node(1), 2);
        f.fail(SIDE + 0x38, 2);
        let refusal = side_failure(&f.sample(&Budget::default()));
        assert_eq!(
            (refusal.cause, refusal.field, refusal.stage),
            (Cause::Unreadable, Field::Head, Stage::Close)
        );
        f.assert_all_closed();
        assert_eq!(f.attempted_bytes(), 288);
    }

    #[test]
    fn parent_open_failures_never_publish_descendants_or_skip_existing_closes() {
        let original = Fixture::new(CombatClass::Land, &[1]);
        for word in original.parent_words() {
            let mut f = Fixture::new(CombatClass::Land, &[1]);
            f.fail(word.at, 1);
            let budget = Budget::default();
            let refusal = parent_failure(&f.sample(&budget));
            assert_eq!(
                (refusal.cause, refusal.field, refusal.stage),
                (Cause::Unreadable, word.field, Stage::Open)
            );
            assert!(!f.visits.contains_key(&(COMBAT + 0x28)));
            assert_eq!(f.attempted_bytes(), budget.verification_bytes());
            f.assert_all_closed();
        }
    }

    #[test]
    fn later_complete_acquisition_recovers_full_census_after_refusal() {
        let mut f = Fixture::new(CombatClass::Land, &[7, 0, -1]);
        f.fail(node(1) + 16, 1);
        assert_eq!(
            side_failure(&f.sample(&Budget::default())).cause,
            Cause::Unreadable
        );
        f.assert_all_closed();

        f.faults.clear();
        f.visits.clear();
        f.events.clear();
        f.chain(&[i32::MIN, 0, 0, i32::MAX], &[3, 1, 0, 2]);
        let result = f.sample(&Budget::default());
        assert_eq!(
            measured(&result).iter().collect::<Vec<_>>(),
            [i32::MIN, 0, 0, i32::MAX]
        );
        f.assert_all_closed();
        // Acquisition-only recovery; the presentation joint exporter covers publication and fold.
    }

    #[test]
    fn portable_word_conversion_funds_whole_new_buffer_without_rereading_or_refunds() {
        for enough in [false, true] {
            let mut f = Fixture::new(CombatClass::Land, &[i32::MIN, 0, -1, 0]);
            let retained = retained_success(4);
            let budget = Budget::test_limits(336, retained + if enough { 16 } else { 15 });
            let result = f.sample(&budget);
            let before_reads = f.events.len();
            let FirstCombatCountries::Present {
                countries: Ok(words),
                ..
            } = result
            else {
                panic!("expected complete acquisition");
            };
            let output = words.try_into_raw_words(&budget);
            if enough {
                assert_eq!(output.unwrap(), [i32::MIN, 0, -1, 0]);
            } else {
                assert_eq!(output.unwrap_err().cause, Cause::RetainedBudgetExceeded);
            }
            assert_eq!(budget.retained_bytes(), retained + 16);
            assert_eq!(f.events.len(), before_reads);
            assert_eq!(budget.verification_bytes(), 336);
            f.assert_all_closed();
        }
    }
}
