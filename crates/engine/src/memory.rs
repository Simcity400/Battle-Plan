use std::mem::{MaybeUninit, size_of};

pub use crate::read_counts::{CountedMemory, ReadCounts};

/// One exact source span and its position in each packed destination record.
/// The same borrowed layout is reused for every source pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct RecordField {
    pub source_offset: usize,
    pub destination_offset: usize,
    pub len: usize,
}

/// Immutable exact-field layout, prepared once before gathering records.
/// Only destination geometry is validated here. Invalid source arithmetic is
/// still reported at its ordered field, after any earlier successful reads.
#[derive(Debug, Clone, Copy)]
pub struct RecordLayout<'a> {
    fields: &'a [RecordField],
    stride: usize,
    envelope: Option<(usize, usize)>,
    nonempty: usize,
    row_bytes: usize,
}

impl<'a> RecordLayout<'a> {
    pub const fn new(fields: &'a [RecordField], stride: usize) -> Option<Self> {
        let mut first = usize::MAX;
        let mut end = 0;
        let mut source_overflow = false;
        let mut nonempty = 0;
        let mut row_bytes = 0_usize;
        let mut index = 0;
        while index < fields.len() {
            let field = &fields[index];
            match field.destination_offset.checked_add(field.len) {
                Some(end) if end <= stride => {}
                _ => return None,
            }
            if field.len != 0 {
                nonempty += 1;
                row_bytes = row_bytes.saturating_add(field.len);
                if field.source_offset < first {
                    first = field.source_offset;
                }
                match field.source_offset.checked_add(field.len) {
                    Some(field_end) => {
                        if field_end > end {
                            end = field_end;
                        }
                    }
                    None => source_overflow = true,
                }
            }
            index += 1;
        }
        Some(Self {
            fields,
            stride,
            envelope: if nonempty != 0 && !source_overflow {
                Some((first, end))
            } else {
                None
            },
            nonempty,
            row_bytes,
        })
    }

    pub const fn fields(&self) -> &'a [RecordField] {
        self.fields
    }

    pub const fn stride(&self) -> usize {
        self.stride
    }

    pub const fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Successful payload bytes for accounting, saturated if the sum overflows.
    /// The ordinary complete-row case does not scan the layout again.
    #[inline(always)]
    pub fn completed_bytes(&self, completed: usize) -> usize {
        if self.fields.is_empty() {
            return 0;
        }
        let full = completed / self.fields.len();
        let tail = completed % self.fields.len();
        self.fields[..tail]
            .iter()
            .fold(full.saturating_mul(self.row_bytes), |bytes, field| {
                bytes.saturating_add(field.len)
            })
    }

    /// Nonempty source requests in a field prefix; complete rows use the
    /// geometry cached at construction, so accounting is O(1) on success.
    #[inline(always)]
    pub fn completed_spans(&self, completed: usize) -> usize {
        if self.fields.is_empty() {
            return 0;
        }
        (completed / self.fields.len())
            .saturating_mul(self.nonempty)
            .saturating_add(
                self.fields[..completed % self.fields.len()]
                    .iter()
                    .filter(|field| field.len != 0)
                    .count(),
            )
    }

    #[inline]
    pub(crate) fn count(
        &self,
        sources: RecordSources<'_>,
        destination_len: usize,
    ) -> Option<usize> {
        let count = sources.len().checked_mul(self.fields.len())?;
        if sources.len().checked_mul(self.stride)? > destination_len {
            return None;
        }
        Some(count)
    }
}

/// Packs `(source_offset, len)` spans back to back into record fields.
pub(crate) const fn packed_fields<const N: usize>(spans: [(usize, usize); N]) -> [RecordField; N] {
    let mut fields = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 0,
    }; N];
    let mut index = 0;
    let mut at = 0;
    while index < N {
        fields[index] = RecordField {
            source_offset: spans[index].0,
            destination_offset: at,
            len: spans[index].1,
        };
        at += spans[index].1;
        index += 1;
    }
    fields
}

/// The record stride of [`packed_fields`].
pub(crate) const fn packed_stride<const N: usize>(spans: [(usize, usize); N]) -> usize {
    let mut index = 0;
    let mut at = 0;
    while index < N {
        at += spans[index].1;
        index += 1;
    }
    at
}

/// The record position of each source byte of [`packed_fields`], or
/// `u16::MAX` for a byte no span covers; `M` is the end of the last span.
/// Spans must ascend without overlapping, so a source range whose two ends map
/// `len - 1` positions apart lies in the record exactly as in the object.
pub(crate) const fn packed_map<const N: usize, const M: usize>(
    spans: [(usize, usize); N],
) -> [u16; M] {
    let mut map = [u16::MAX; M];
    let mut index = 0;
    let mut at = 0;
    let mut end = 0;
    while index < N {
        let (source, len) = spans[index];
        assert!(source >= end && len != 0, "packed spans must ascend");
        assert!(at + len < u16::MAX as usize, "packed record too long");
        let mut byte = 0;
        while byte < len {
            map[source + byte] = (at + byte) as u16;
            byte += 1;
        }
        at += len;
        end = source + len;
        index += 1;
    }
    assert!(end == M, "map must end with the last span");
    map
}

/// Gathers one record per pointer. `complete[row]` is set only when every
/// field of that row copied. A failed row does not stop later rows: the copy
/// resumes after it, so each object is read by one guarded record request
/// and a failed object by at most one.
pub(crate) fn gather_records(
    memory: &impl Memory,
    pointers: &[usize],
    layout: &RecordLayout<'_>,
    bytes: &mut [u8],
    complete: &mut [bool],
) {
    let fields = layout.field_count().max(1);
    let stride = layout.stride();
    let rows = pointers.len();
    let mut row = 0;
    while row < rows {
        let copied = memory.copy_records(
            RecordSources::Pointers(&pointers[row..]),
            layout,
            &mut bytes[row * stride..rows * stride],
        );
        let done = (copied / fields).min(rows - row);
        complete[row..row + done].fill(true);
        row += done;
        if row < rows {
            complete[row] = false;
            row += 1;
        }
    }
}

/// One object's fields from [`gather_records`], or nothing when its record
/// did not complete. Readers take a value from here first and otherwise make
/// their original per-field read, so rows and failures are those of the
/// per-field walk: every gathered byte is the byte that read would copy.
#[derive(Clone, Copy)]
pub(crate) struct Gathered<'a> {
    /// The layout's [`packed_map`].
    map: &'a [u16],
    bytes: Option<&'a [u8]>,
}

impl<'a> Gathered<'a> {
    pub(crate) const NONE: Gathered<'static> = Gathered {
        map: &[],
        bytes: None,
    };

    #[inline(always)]
    pub(crate) fn row(
        layout: &RecordLayout<'a>,
        map: &'a [u16],
        bytes: &'a [u8],
        complete: &[bool],
        row: usize,
    ) -> Self {
        let stride = layout.stride();
        Self {
            map,
            bytes: complete
                .get(row)
                .copied()
                .unwrap_or(false)
                .then(|| &bytes[row * stride..(row + 1) * stride]),
        }
    }

    /// The gathered value at `offset`, if the record completed and its spans
    /// cover every byte of it. The map is constant, so a constant offset
    /// resolves at compile time.
    #[inline(always)]
    pub(crate) fn get<T: Raw>(self, offset: usize) -> Option<T> {
        let bytes = self.bytes?;
        let last = size_of::<T>().checked_sub(1)?;
        let first = *self.map.get(offset)?;
        let end = *self.map.get(offset.checked_add(last)?)?;
        if first == u16::MAX || end == u16::MAX || usize::from(end.wrapping_sub(first)) != last {
            return None;
        }
        let at = usize::from(first);
        scalar_from_bytes(bytes.get(at..=at + last)?)
    }
}

/// Current source addresses, either explicit pointers or an exact strided array.
/// Strided addresses are evaluated only when a nonempty field requires them.
#[derive(Debug, Clone, Copy)]
pub enum RecordSources<'a> {
    Pointers(&'a [usize]),
    Strided {
        base: usize,
        stride: usize,
        count: usize,
    },
}

impl RecordSources<'_> {
    pub const fn len(self) -> usize {
        match self {
            Self::Pointers(pointers) => pointers.len(),
            Self::Strided { count, .. } => count,
        }
    }

    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn pointer(self, row: usize) -> Option<usize> {
        match self {
            Self::Pointers(pointers) => pointers.get(row).copied(),
            Self::Strided {
                base,
                stride,
                count,
            } if row < count => base.checked_add(row.checked_mul(stride)?),
            Self::Strided { .. } => None,
        }
    }
}

/// x86-64 canonical range check for a guarded source span.
///
/// A user address is canonical when bits 63:48 equal bit 47 (low half
/// `0x0000_...` or high half `0xFFFF_...`). Windows reports a fault on a
/// noncanonical source as `ExceptionInformation[1] == -1`, which the C
/// `copy_fault` filter cannot attribute to the requested range and would
/// otherwise return `EXCEPTION_CONTINUE_SEARCH` (process crash). Gating here
/// records the same graceful `false`/prefix failure the typed walk produces
/// for an unreadable source, before any guarded instruction runs.
///
/// Empty spans succeed without a check, matching `Memory`. Overflow is
/// reported by the caller's existing null/overflow path; this helper returns
/// `false` for it as well so callers record a fault without touching memory.
/// Both range ends must be canonical and in the same half; a span crossing
/// the noncanonical gap is noncanonical even when both ends look canonical
/// in isolation.
#[inline]
pub(crate) fn is_canonical_range(addr: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let Some(end) = addr.checked_add(len.wrapping_sub(1)) else {
        return false;
    };
    // Bit 47 is part of the sign extension, not an independent address bit.
    let high = addr >> 47;
    matches!(high, 0 | 0x1FFFF) && end >> 47 == high
}

/// Return the row-major prefix whose nonempty source spans have valid address
/// arithmetic and are canonical. `RecordLayout::count` validates the complete
/// count first. This reads immutable metadata only, never any source bytes.
#[inline(always)]
fn record_source_prefix(sources: RecordSources<'_>, layout: &RecordLayout<'_>) -> usize {
    if layout.nonempty == 0 || sources.is_empty() {
        return sources.len() * layout.fields.len();
    }
    // A valid union envelope proves every address in a strided batch at once.
    // It does not read source gaps or prove readability. Failed envelopes fall
    // back to field order, preserving partial rows and earlier actual faults.
    if matches!(sources, RecordSources::Strided { .. })
        && let Some((first, end)) = layout.envelope
        && let Some(base) = sources.pointer(0).filter(|&pointer| pointer != 0)
        && let Some(last) = sources.pointer(sources.len() - 1)
        && let Some(last_end) = last.checked_add(end)
        && let Some(first_start) = base.checked_add(first)
        && is_canonical_range(first_start, last_end - first_start)
    {
        return sources.len() * layout.fields.len();
    }
    let mut completed = 0;
    for row in 0..sources.len() {
        let pointer = sources.pointer(row);
        if pointer.is_some_and(|pointer| {
            pointer != 0
                && layout.envelope.is_some_and(|(first, end)| {
                    pointer.checked_add(end).is_some()
                        && is_canonical_range(pointer + first, end - first)
                })
        }) {
            // Every nonempty field is inside this canonical, nonoverflowing
            // range. Holes are not read; readability is still decided by SEH
            // in field order. A failed envelope proves nothing about a prefix.
            completed += layout.fields.len();
            continue;
        }
        for field in layout.fields {
            if field.len != 0 {
                let Some(address) = pointer
                    .filter(|&pointer| pointer != 0)
                    .and_then(|pointer| pointer.checked_add(field.source_offset))
                    .filter(|&address| address.checked_add(field.len).is_some())
                else {
                    return completed;
                };
                if !is_canonical_range(address, field.len) {
                    return completed;
                }
            }
            completed += 1;
        }
    }
    completed
}

/// Implementations fill the destination or return false; a failed copy may be partial.
pub trait Memory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool;

    /// [`copy`](Self::copy) into scratch bytes that need not be initialized
    /// first. On success every byte of `destination` holds its source byte and
    /// is returned initialized; on failure nothing is returned, because a
    /// failed copy may be partial and no byte of it may be read. The same
    /// observation as `copy`: the default initializes the destination and
    /// calls `copy`, so every adapter keeps its own caching, recording and
    /// accounting. Leaf sources that can write uninitialized bytes override it.
    #[inline]
    fn copy_uninit<'a>(
        &self,
        address: usize,
        destination: &'a mut [MaybeUninit<u8>],
    ) -> Option<&'a mut [u8]> {
        let destination = initialized(destination);
        self.copy(address, destination).then_some(destination)
    }

    /// Advisory hint for an already selected source. No value is acquired;
    /// recording adapters may retain the hint with its selecting dependencies.
    #[inline]
    fn prefetch(&self, address: usize, len: usize) {
        crate::prefetch::span(address, len);
    }

    /// Acquire one value with the same observation and failure semantics as
    /// `copy`. Adapters must retain their own caching, recording and accounting
    /// before forwarding this operation. The default uses their existing copy.
    #[inline]
    fn read_scalar<T: Raw>(&self, address: usize) -> Option<T> {
        copy_scalar(self, address)
    }

    /// Execute prepared scalar reads and owned-control comparisons in order.
    /// The default preserves this adapter's `copy` observations. `None` means
    /// unsupported execution or insufficient destination geometry, never a
    /// changed control/read fault. Wrappers must not forward to a different
    /// source implementation unless their acquisition protocol permits it.
    fn execute_read_plan(
        &self,
        plan: &crate::read_plan::PreparedPlan,
        destination: &mut [u8],
    ) -> Option<crate::read_plan::Outcome> {
        plan.execute(self, destination)
    }

    /// Optional immutable, already-owned bytes for this exact address range.
    /// Every contained nonempty read must equal `copy` without consuming any
    /// observation cursor. Implementations must exclude overriding failures,
    /// repeated observations and competing bulk ranges, not merely find a
    /// readable enclosing allocation. The borrow never exposes live memory.
    /// Observation-recording wrappers must not forward this capability.
    fn immutable_view(&self, _address: usize, _len: usize) -> Option<SliceMemory<'_>> {
        None
    }

    /// Optional authentication derived from this callback's complete country
    /// census. The view must belong to the exact module/game-state pair and
    /// retain its initial authentication only within the quiescent hourly lease.
    /// Observation-recording wrappers must not forward this capability: their
    /// readers still need every original byte request in order.
    fn authenticated_countries(
        &self,
        _base: usize,
        _game_state: usize,
    ) -> Option<crate::ccountry::AuthenticatedRoster<'_>> {
        None
    }

    /// Optional raw lookup from a complete, sorted accumulator snapshot owned
    /// by this acquisition pass. Its header and entries come from the current
    /// source acquisition and remain owned for the entire lookup.
    /// Absence of this capability preserves the ordinary live reader. This
    /// hook must not evaluate modifier metadata or escape its pass lifetime;
    /// observation-recording wrappers must not forward it.
    fn modifier_accumulator_value(
        &self,
        _address: usize,
        _key: u32,
    ) -> Option<Result<crate::modifier_accumulator::Value, &'static str>> {
        None
    }

    /// Optional exact core-supply predicate shared only within an acquisition
    /// pass. All keys and results are acquired from the current source and
    /// cleared between passes; collisions/capacity use the ordinary
    /// reader. Observation-recording wrappers must not forward this capability.
    fn core_supply_access(
        &self,
        _base: usize,
        _game_state: usize,
        _owner_raw: u32,
        _area: usize,
    ) -> Option<Result<bool, &'static str>> {
        None
    }

    /// Optional primary-order membership dependency cache for one ETA pass.
    /// Own/parent first-match traversal, dirty/clean cache checks and read-lazy
    /// prefixes must be preserved. Retained metadata, vector headers and chunks
    /// belong to the current acquisition and are cleared between passes.
    /// This is not has_order.
    /// Observation-recording wrappers must not forward this capability.
    fn movement_primary_root(
        &self,
        _base: usize,
        _member: usize,
    ) -> Option<Result<usize, &'static str>> {
        None
    }

    /// Optional deterministic arithmetic memo, keyed by every raw scalar
    /// argument. Callers must still acquire/authenticate those arguments on
    /// every invocation. This
    /// capability never supplies game-derived inputs or changes lazy reads.
    /// Observation-recording wrappers must not forward it.
    fn movement_air_defense_value(
        &self,
        _defense: i64,
        _denominator: i64,
        _reduction: i64,
    ) -> Option<Result<i64, &'static str>> {
        None
    }

    /// Copies fields in order and returns the number fully copied before the first
    /// failure. The failed field may be partial; later destinations are untouched.
    /// Empty fields count as complete without accessing their source address.
    /// The caller owns every buffer; no allocation or retry occurs.
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        for (completed, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if destination.is_empty() {
                continue;
            }
            if address == 0
                || address.checked_add(destination.len()).is_none()
                || !self.copy(address, destination)
            {
                return completed;
            }
        }
        N
    }

    /// The same exact ordered scattered copy with advisory cache hints for its
    /// independent source spans. Callers should supply bounded batches of fields
    /// whose addresses are already known. Hints neither read values nor consume
    /// guard pages; the first failed copy still leaves every later destination
    /// untouched. No sorting, allocation, retry or retained observation occurs.
    ///
    /// Adapters retain their existing `copy_scattered` implementation, including
    /// read logging and cached-source rules. LiveMemory rolls a four-span hint
    /// window inside the native guard; adapters hint this batch before forwarding
    /// through their own scattered-copy protocol.
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        for (&address, destination) in addresses.iter().zip(destinations.iter()) {
            if destination.is_empty() {
                continue;
            }
            if address == 0 || address.checked_add(destination.len()).is_none() {
                break;
            }
            crate::prefetch::span(address, destination.len());
        }
        self.copy_scattered(addresses, destinations)
    }

    /// Gather a fixed exact layout from independently addressed records into
    /// initialized caller-owned bytes. Returns completed fields in row-major
    /// order; a failed field may be partial and later fields stay untouched.
    /// Empty fields count as complete without reading their source. Invalid
    /// destination bounds/count arithmetic returns zero before any access;
    /// null or overflowing source addresses stop at that particular field.
    /// Destination bytes outside the declared spans are left alone. No
    /// allocation, source padding read, retry or observation cache is involved.
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let Some(count) = layout.count(sources, destination.len()) else {
            return 0;
        };
        if count == 0 || layout.nonempty == 0 {
            return count;
        }
        let mut completed = 0;
        for row in 0..sources.len() {
            for field in layout.fields {
                if field.len != 0 {
                    let Some(address) = sources
                        .pointer(row)
                        .filter(|&pointer| pointer != 0)
                        .and_then(|pointer| pointer.checked_add(field.source_offset))
                        .filter(|&address| address.checked_add(field.len).is_some())
                    else {
                        return completed;
                    };
                    let start = row * layout.stride + field.destination_offset;
                    if !self.copy(address, &mut destination[start..start + field.len]) {
                        return completed;
                    }
                }
                completed += 1;
            }
        }
        completed
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Padding-free types for which every bit pattern is valid. Sealed to prevent
/// decoding pointers, references, bools, or enums with invalid representations.
pub trait Raw: sealed::Sealed + Copy {
    const ZERO: Self;
}

macro_rules! raw_integers {
    ($($ty:ty),* $(,)?) => { $(
        impl sealed::Sealed for $ty {}
        impl Raw for $ty { const ZERO: Self = 0; }
    )* };
}
raw_integers!(u8, i8, u16, i16, u32, i32, u64, i64);
impl<const N: usize> sealed::Sealed for [u8; N] {}
impl<const N: usize> Raw for [u8; N] {
    const ZERO: Self = [0; N];
}

#[inline]
fn copy_scalar<T: Raw>(memory: &(impl Memory + ?Sized), address: usize) -> Option<T> {
    let mut value = T::ZERO;
    // SAFETY: Raw is sealed to initialized, padding-free, all-bit-pattern types.
    let bytes = unsafe {
        std::slice::from_raw_parts_mut((&mut value as *mut T).cast::<u8>(), size_of::<T>())
    };
    memory.copy(address, bytes).then_some(value)
}

/// The initialized exact bytes of a padding-free scalar or byte array.
#[inline]
pub fn scalar_bytes<T: Raw>(value: &T) -> &[u8] {
    // SAFETY: Raw is sealed to initialized, padding-free, all-bit-pattern types.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Reconstruct a scalar without a numeric conversion or an extra observation.
#[inline]
pub fn scalar_from_bytes<T: Raw>(source: &[u8]) -> Option<T> {
    if source.len() != size_of::<T>() {
        return None;
    }
    let mut value = T::ZERO;
    // SAFETY: Raw has no padding or invalid bit patterns; the exact destination
    // is initialized, writable and cannot overlap the borrowed source.
    let destination = unsafe {
        std::slice::from_raw_parts_mut((&mut value as *mut T).cast::<u8>(), size_of::<T>())
    };
    destination.copy_from_slice(source);
    Some(value)
}

#[inline]
pub fn read<T: Raw>(memory: &impl Memory, address: usize) -> Option<T> {
    let len = size_of::<T>();
    if len == 0 {
        return Some(T::ZERO);
    }
    if address == 0 || address.checked_add(len).is_none() {
        return None;
    }
    if len == 4 {
        memory.read_scalar(address)
    } else {
        copy_scalar(memory, address)
    }
}

/// The caller owns the buffer. No allocation occurs, including on a read failure.
#[inline(always)]
pub fn read_slice(memory: &impl Memory, address: usize, destination: &mut [u8]) -> bool {
    if destination.is_empty() {
        return true;
    }
    address != 0
        && address.checked_add(destination.len()).is_some()
        && memory.copy(address, destination)
}

/// [`read_slice`] into uninitialized scratch: the same checks and the same
/// single copy, returning the bytes only when every one of them was copied.
#[inline]
pub(crate) fn read_slice_uninit<'a>(
    memory: &impl Memory,
    address: usize,
    destination: &'a mut [MaybeUninit<u8>],
) -> Option<&'a mut [u8]> {
    if destination.is_empty() {
        return Some(initialized(destination));
    }
    if address == 0 || address.checked_add(destination.len()).is_none() {
        return None;
    }
    memory.copy_uninit(address, destination)
}

/// Zeroes `bytes` and returns them initialized.
#[inline]
fn initialized(bytes: &mut [MaybeUninit<u8>]) -> &mut [u8] {
    for byte in bytes.iter_mut() {
        byte.write(0);
    }
    // SAFETY: every byte was written just above, and `u8` has the layout of
    // `MaybeUninit<u8>`; the borrow is the caller's unique borrow.
    unsafe { &mut *(std::ptr::from_mut(bytes) as *mut [u8]) }
}

/// An immutable address space backed by borrowed, already-owned bytes.
pub struct SliceMemory<'a> {
    pub base: usize,
    pub bytes: &'a [u8],
}

impl Memory for SliceMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let Some(start) = address.checked_sub(self.base) else {
            return false;
        };
        let Some(end) = start.checked_add(destination.len()) else {
            return false;
        };
        let Some(source) = self.bytes.get(start..end) else {
            return false;
        };
        destination.copy_from_slice(source);
        true
    }

    fn copy_uninit<'a>(
        &self,
        address: usize,
        destination: &'a mut [MaybeUninit<u8>],
    ) -> Option<&'a mut [u8]> {
        let start = address.checked_sub(self.base)?;
        let source = self
            .bytes
            .get(start..start.checked_add(destination.len())?)?;
        // SAFETY: `source` and `destination` have the same length and cannot
        // overlap (one is borrowed shared, the other unique). The copy writes
        // every destination byte, so all of them are initialized after it.
        unsafe {
            std::ptr::copy_nonoverlapping(
                source.as_ptr(),
                destination.as_mut_ptr().cast::<u8>(),
                source.len(),
            );
            Some(&mut *(std::ptr::from_mut(destination) as *mut [u8]))
        }
    }

    /// Hints the slice bytes that back `address`, not `address` itself: the
    /// default would hint an unrelated host address. Bytes outside the slice
    /// are not hinted.
    #[inline]
    fn prefetch(&self, address: usize, len: usize) {
        if let Some(start) = address.checked_sub(self.base)
            && let Some(rest) = self.bytes.get(start..)
        {
            crate::prefetch::span(rest.as_ptr() as usize, len.min(rest.len()));
        }
    }
}

#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
pub struct LiveMemory(std::marker::PhantomData<std::rc::Rc<()>>);

#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
impl LiveMemory {
    /// # Safety
    /// Use only for foreign game memory under the caller's consistency protocol,
    /// or immutable synthetic storage in tests.
    /// SEH protects against invalid addresses, not concurrent writes or Rust aliasing violations.
    /// This handle is deliberately neither Send nor Sync.
    pub unsafe fn new() -> Self {
        Self(std::marker::PhantomData)
    }

    #[inline(always)]
    fn copy_scattered_inner<const N: usize, const PREFETCH: bool>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        #[derive(Clone, Copy)]
        #[repr(C)]
        struct Span {
            dst: *mut u8,
            src: *const u8,
            len: usize,
        }
        unsafe extern "C" {
            fn seh_guarded_copy_scattered(spans: *const Span, count: usize) -> usize;
            fn seh_guarded_copy_scattered_prefetched(spans: *const Span, count: usize) -> usize;
        }
        // Only the validated prefix crosses the C boundary. Avoid clearing the
        // entire metadata array before replacing that prefix, especially for
        // large batches with an invalid address or a short populated tail.
        let mut spans = [std::mem::MaybeUninit::<Span>::uninit(); N];
        let mut count = 0;
        let mut copy_count = 0;
        for (index, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if !destination.is_empty()
                && (address == 0
                    || address.checked_add(destination.len()).is_none()
                    || !is_canonical_range(address, destination.len()))
            {
                break;
            }
            spans[index].write(Span {
                dst: destination.as_mut_ptr(),
                src: address as *const u8,
                len: destination.len(),
            });
            count = index + 1;
            if !destination.is_empty() {
                copy_count = count;
            }
        }
        if copy_count == 0 {
            return count;
        }
        // SAFETY: spans[..count] is initialized, including empty entries, and
        // metadata and writable destinations outlive C. Both native paths copy
        // the same exact spans in order, returning the completed prefix on a
        // fault. The opt-in path only hints validated future spans; it performs
        // no future loads, and hints cannot consume a guard-page exception.
        let copied = unsafe {
            if PREFETCH {
                seh_guarded_copy_scattered_prefetched(spans.as_ptr().cast::<Span>(), copy_count)
            } else {
                seh_guarded_copy_scattered(spans.as_ptr().cast::<Span>(), copy_count)
            }
        };
        if copied == copy_count {
            // Trailing empty spans succeed without entering the C loop. Empty
            // spans before a failure retain their original prefix positions.
            count
        } else {
            copied
        }
    }
}

/// One guarded copy of `len` foreign bytes from `address` to `destination`,
/// true only when all of them were copied; a fault may leave a written prefix.
///
/// # Safety
/// `destination` must be valid for `len` byte writes, and `address..+len`
/// must have passed the null, overflow and canonical gates.
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
#[inline(always)]
unsafe fn guarded_copy(destination: *mut u8, address: usize, len: usize) -> bool {
    unsafe extern "C" {
        fn seh_guarded_copy(dst: *mut u8, src: *const u8, len: usize) -> i32;
        fn seh_guarded_copy4(dst: *mut u8, src: *const u8) -> i32;
        fn seh_guarded_copy8(dst: *mut u8, src: *const u8) -> i32;
    }
    // SAFETY: the caller's contract; C catches faults in the source range.
    // Inlined scalar readers fold this selection to a fixed-width call.
    let copied = unsafe {
        match len {
            8 => seh_guarded_copy8(destination, address as *const u8),
            4 => seh_guarded_copy4(destination, address as *const u8),
            len => seh_guarded_copy(destination, address as *const u8, len),
        }
    };
    copied != 0
}

#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
impl Memory for LiveMemory {
    #[inline(always)]
    fn read_scalar<T: Raw>(&self, address: usize) -> Option<T> {
        if size_of::<T>() != 4 {
            return copy_scalar(self, address);
        }
        if address == 0 || address.checked_add(4).is_none() || !is_canonical_range(address, 4) {
            return None;
        }
        unsafe extern "C" {
            fn seh_guarded_read4(src: *const u8) -> u64;
        }
        // SAFETY: C guards exactly the admitted four-byte foreign source. Its
        // upper word is one on success and zero on failure, independently of
        // every possible lower-word value. Failed loads publish no value.
        let packed = unsafe { seh_guarded_read4(address as *const u8) };
        if packed >> 32 == 0 {
            return None;
        }
        scalar_from_bytes(&(packed as u32).to_ne_bytes())
    }

    fn execute_read_plan(
        &self,
        plan: &crate::read_plan::PreparedPlan,
        destination: &mut [u8],
    ) -> Option<crate::read_plan::Outcome> {
        use crate::read_plan::{NativeControl, NativeHint, NativeRead, Outcome};

        unsafe extern "C" {
            fn seh_execute_read_plan(
                reads: *const NativeRead,
                read_count: usize,
                controls: *const NativeControl,
                control_count: usize,
                hints: *const NativeHint,
                hint_count: usize,
                expected: *const u8,
                destination: *mut u8,
                completed: *mut usize,
            ) -> i32;
        }
        if destination.len() < plan.destination_len() {
            return None;
        }
        let Some(native) = plan.native() else {
            return plan.execute(self, destination);
        };
        let mut completed = 0;
        // SAFETY: PreparedPlan owns immutable repr(C) descriptors and expected
        // bytes; its constructor validates every source and destination bound.
        // The writable owned destination outlives C, which guards only the
        // active exact source copy and stops before every later read on failure.
        let result = unsafe {
            seh_execute_read_plan(
                native.reads.as_ptr(),
                native.reads.len(),
                native.controls.as_ptr(),
                native.controls.len(),
                native.hints.as_ptr(),
                native.hints.len(),
                plan.expected().as_ptr(),
                destination.as_mut_ptr(),
                &mut completed,
            )
        };
        Some(match result {
            0 => Outcome::Complete,
            1 => Outcome::ControlChanged {
                completed_reads: completed,
            },
            2 => Outcome::ReadFailed {
                completed_reads: completed,
            },
            _ => unreachable!("native read plan returned an invalid outcome"),
        })
    }

    #[inline(always)]
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return true;
        }
        if address == 0 || address.checked_add(destination.len()).is_none() {
            return false;
        }
        // Noncanonical sources fault with ExceptionInformation[1] == -1, which
        // the C filter cannot attribute to the requested range. Fail here with
        // the same `false` an unreadable canonical source produces; the typed
        // walk then reports its own site-specific reason instead of crashing.
        if !is_canonical_range(address, destination.len()) {
            return false;
        }
        // SAFETY: destination is writable; C catches faults in the foreign source range.
        unsafe { guarded_copy(destination.as_mut_ptr(), address, destination.len()) }
    }

    #[inline(always)]
    fn copy_uninit<'a>(
        &self,
        address: usize,
        destination: &'a mut [MaybeUninit<u8>],
    ) -> Option<&'a mut [u8]> {
        if destination.is_empty() {
            return Some(initialized(destination));
        }
        // The same gates as `copy`, before any guarded instruction.
        if address == 0
            || address.checked_add(destination.len()).is_none()
            || !is_canonical_range(address, destination.len())
        {
            return None;
        }
        // SAFETY: the destination pointer covers `len` writable bytes the
        // caller borrows uniquely; C only stores to them and never reads them.
        // C returns nonzero only when it copied the whole source range, so
        // then every destination byte is initialized. On a fault it returns
        // zero, possibly after storing a prefix, and no byte is exposed.
        unsafe {
            guarded_copy(
                destination.as_mut_ptr().cast::<u8>(),
                address,
                destination.len(),
            )
            .then(|| &mut *(std::ptr::from_mut(destination) as *mut [u8]))
        }
    }

    #[inline(always)]
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.copy_scattered_inner::<N, false>(addresses, destinations)
    }

    #[inline(always)]
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.copy_scattered_inner::<N, true>(addresses, destinations)
    }

    #[inline(always)]
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        unsafe extern "C" {
            fn seh_guarded_copy_records(
                pointers: *const usize,
                source_base: usize,
                source_stride: usize,
                validated_field_count: usize,
                fields: *const RecordField,
                field_count: usize,
                row_stride: usize,
                destination: *mut u8,
            ) -> usize;
        }
        let Some(count) = layout.count(sources, destination.len()) else {
            return 0;
        };
        if count == 0 || layout.nonempty == 0 {
            return count;
        }
        let validated_field_count = record_source_prefix(sources, layout);
        if validated_field_count == 0 {
            return 0;
        }
        // SAFETY: the two immutable metadata slices and initialized writable
        // destination outlive the call. RecordField has the C layout, and the
        // checked row count/stride and field bounds cover every destination.
        // Every nonempty source within validated_field_count has passed the
        // Rust null/overflow/canonical checks. C only evaluates that prefix,
        // including a possible partial final row; invalid and later sources
        // never reach a guarded instruction. SEH still decides readability in
        // order, so an earlier unreadable field wins over a later invalid one.
        // C preserves its completed-field witness and exact fault-range filter.
        let (pointers, source_base, source_stride) = match sources {
            RecordSources::Pointers(pointers) => (pointers.as_ptr(), 0, 0),
            RecordSources::Strided { base, stride, .. } => (std::ptr::null(), base, stride),
        };
        unsafe {
            seh_guarded_copy_records(
                pointers,
                source_base,
                source_stride,
                validated_field_count,
                layout.fields.as_ptr(),
                layout.fields.len(),
                layout.stride,
                destination.as_mut_ptr(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    struct TracedMemory {
        calls: RefCell<Vec<usize>>,
        fail_at: usize,
    }

    impl Memory for TracedMemory {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.borrow_mut().push(address);
            if address == self.fail_at {
                destination[0] = 0x7e;
                false
            } else {
                destination.fill(address as u8);
                true
            }
        }
    }

    #[test]
    fn record_source_envelope_never_rejects_fields_across_a_canonical_gap() {
        let fields = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 4,
            },
            RecordField {
                source_offset: 0xffff_8000_0000_0000 - 1,
                destination_offset: 4,
                len: 4,
            },
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 8,
                len: 0,
            },
        ];
        let (first, end) = RecordLayout::new(&fields, 8).unwrap().envelope.unwrap();
        assert!(!is_canonical_range(1 + first, end - first));
        // No reads occur here. Both nonempty fields are canonical even though
        // the conservative envelope crosses the noncanonical gap between them.
        assert_eq!(
            record_source_prefix(
                RecordSources::Pointers(&[1]),
                &RecordLayout::new(&fields, usize::MAX).unwrap()
            ),
            3
        );
        assert_eq!(
            record_source_prefix(
                RecordSources::Pointers(&[1]),
                &RecordLayout::new(&[fields[1], fields[0], fields[2]], usize::MAX).unwrap()
            ),
            3
        );
        assert_eq!(
            RecordLayout::new(&fields[..1], 8).unwrap().envelope,
            Some((0, 4))
        );
        assert_eq!(
            RecordLayout::new(&[fields[0], fields[2]], 8)
                .unwrap()
                .envelope,
            Some((0, 4))
        );
        assert_eq!(
            record_source_prefix(
                RecordSources::Pointers(&[0, usize::MAX]),
                &RecordLayout::new(&fields[2..], usize::MAX).unwrap()
            ),
            2
        );
        assert_eq!(
            record_source_prefix(
                RecordSources::Pointers(&[]),
                &RecordLayout::new(&fields, usize::MAX).unwrap()
            ),
            0
        );
    }

    #[test]
    fn record_source_prefix_matches_ordered_scan_at_address_boundaries() {
        fn ordered(pointers: &[usize], fields: &[RecordField]) -> usize {
            let mut completed = 0;
            for &pointer in pointers {
                for field in fields {
                    if field.len != 0 {
                        let valid = pointer != 0
                            && pointer
                                .checked_add(field.source_offset)
                                .is_some_and(|address| {
                                    address.checked_add(field.len).is_some()
                                        && is_canonical_range(address, field.len)
                                });
                        if !valid {
                            return completed;
                        }
                    }
                    completed += 1;
                }
            }
            completed
        }
        let addresses = [
            0,
            1,
            0x1000,
            0x0000_7fff_ffff_fff0,
            0x0000_8000_0000_0000,
            0xffff_7fff_ffff_ffff,
            0xffff_8000_0000_0000,
            usize::MAX - 3,
            usize::MAX,
        ];
        for first_offset in [0, 7, 0x0000_8000_0000_0000, usize::MAX - 1] {
            for second_offset in [0, 31, 0xffff_8000_0000_0000, usize::MAX] {
                for first_len in [0, 1, 8, 32] {
                    for second_len in [0, 1, 8, 32] {
                        let fields = [
                            RecordField {
                                source_offset: first_offset,
                                destination_offset: 0,
                                len: first_len,
                            },
                            RecordField {
                                source_offset: usize::MAX,
                                destination_offset: 32,
                                len: 0,
                            },
                            RecordField {
                                source_offset: second_offset,
                                destination_offset: 32,
                                len: second_len,
                            },
                        ];
                        for pointer in addresses {
                            let pointers = [0x1000, pointer, 0x1000];
                            assert_eq!(
                                record_source_prefix(
                                    RecordSources::Pointers(&pointers),
                                    &RecordLayout::new(&fields, usize::MAX).unwrap()
                                ),
                                ordered(&pointers, &fields),
                                "pointers={pointers:?}, fields={fields:?}"
                            );
                            for stride in [0, 8, 0x0000_8000_0000_0000, usize::MAX] {
                                let sources = RecordSources::Strided {
                                    base: pointer,
                                    stride,
                                    count: 3,
                                };
                                let pointers = std::array::from_fn::<_, 3, _>(|row| {
                                    sources.pointer(row).unwrap_or(0)
                                });
                                assert_eq!(
                                    record_source_prefix(
                                        sources,
                                        &RecordLayout::new(&fields, 64).unwrap()
                                    ),
                                    ordered(&pointers, &fields),
                                    "base={pointer:x}, stride={stride:x}, fields={fields:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn strided_records_keep_empty_overflow_and_repeated_observations() {
        const FIELDS: [RecordField; 3] = [
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 0,
                len: 0,
            },
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 4,
            },
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 4,
                len: 0,
            },
        ];
        const LAYOUT: RecordLayout<'static> = RecordLayout::new(&FIELDS, 4).unwrap();
        let overflow = RecordSources::Strided {
            base: 0x1000,
            stride: usize::MAX - 0xfff,
            count: 2,
        };
        for fail_at in [0, 0x1000] {
            let memory = TracedMemory {
                calls: RefCell::new(Vec::new()),
                fail_at,
            };
            let mut output = [0xa5; 8];
            let copied = memory.copy_records(overflow, &LAYOUT, &mut output);
            assert_eq!(copied, if fail_at == 0 { 4 } else { 1 });
            assert_eq!(*memory.calls.borrow(), [0x1000]);
            assert_eq!(&output[4..], &[0xa5; 4]);
            assert_eq!(
                LAYOUT.completed_bytes(copied),
                if fail_at == 0 { 4 } else { 0 }
            );
            assert_eq!(LAYOUT.completed_spans(copied), usize::from(fail_at == 0));
        }
        let memory = TracedMemory {
            calls: RefCell::new(Vec::new()),
            fail_at: 0,
        };
        assert_eq!(
            memory.copy_records(
                RecordSources::Strided {
                    base: 0x1000,
                    stride: 0,
                    count: 2
                },
                &LAYOUT,
                &mut [0; 8]
            ),
            6
        );
        assert_eq!(*memory.calls.borrow(), [0x1000, 0x1000]);

        // A synthetic Memory retains its own numeric-address space. Only the
        // LiveMemory implementation imposes host canonical-address checks.
        memory.calls.borrow_mut().clear();
        assert_eq!(
            memory.copy_records(
                RecordSources::Strided {
                    base: 0x0000_8000_0000_0000,
                    stride: 0,
                    count: 1
                },
                &LAYOUT,
                &mut [0; 4]
            ),
            3
        );
        assert_eq!(*memory.calls.borrow(), [0x0000_8000_0000_0000]);

        let empty = [FIELDS[0]];
        let empty_layout = RecordLayout::new(&empty, 0).unwrap();
        memory.calls.borrow_mut().clear();
        assert_eq!(
            memory.copy_records(
                RecordSources::Strided {
                    base: 0,
                    stride: usize::MAX,
                    count: usize::MAX
                },
                &empty_layout,
                &mut []
            ),
            usize::MAX
        );
        assert!(memory.calls.borrow().is_empty());
    }

    #[test]
    fn records_copy_exact_fields_and_keep_failed_prefix_and_padding() {
        let fields = [
            RecordField {
                source_offset: 1,
                destination_offset: 0,
                len: 2,
            },
            RecordField {
                source_offset: 0x20,
                destination_offset: 4,
                len: 4,
            },
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 8,
                len: 0,
            },
            RecordField {
                source_offset: 0x38,
                destination_offset: 8,
                len: 8,
            },
        ];
        let memory = TracedMemory {
            calls: RefCell::new(Vec::new()),
            fail_at: 0x1120,
        };
        let mut bytes = [0xa5; 48];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[0x1000, 0x1100]),
                &RecordLayout::new(&fields, 20).expect("valid record layout"),
                &mut bytes
            ),
            5
        );
        assert_eq!(
            *memory.calls.borrow(),
            [0x1001, 0x1020, 0x1038, 0x1101, 0x1120]
        );
        let mut expected = [0xa5; 48];
        expected[..2].fill(1);
        expected[4..8].fill(0x20);
        expected[8..16].fill(0x38);
        expected[20..22].fill(1);
        expected[24] = 0x7e;
        assert_eq!(bytes, expected);
    }

    #[test]
    fn record_destination_bounds_fail_before_access_and_sources_fail_in_order() {
        let memory = TracedMemory {
            calls: RefCell::new(Vec::new()),
            fail_at: 0,
        };
        for (field, stride) in [
            (
                RecordField {
                    source_offset: 0,
                    destination_offset: 7,
                    len: 2,
                },
                8,
            ),
            (
                RecordField {
                    source_offset: 0,
                    destination_offset: usize::MAX,
                    len: 1,
                },
                8,
            ),
            (
                RecordField {
                    source_offset: 0,
                    destination_offset: 0,
                    len: 1,
                },
                usize::MAX,
            ),
        ] {
            let mut bytes = [0xa5; 16];
            let fields = [field];
            let layout = RecordLayout::new(&fields, stride);
            assert_eq!(
                layout.as_ref().map_or(0, |layout| memory.copy_records(
                    RecordSources::Pointers(&[0x1000, 0x1100]),
                    layout,
                    &mut bytes
                )),
                0
            );
            assert_eq!(bytes, [0xa5; 16]);
            #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
            {
                let source = [0x6d; 16];
                // SAFETY: sources are immutable owned bytes; invalid
                // destination geometry must be rejected before native entry.
                let live = unsafe { LiveMemory::new() };
                assert_eq!(
                    layout.as_ref().map_or(0, |layout| live.copy_records(
                        RecordSources::Pointers(&[source.as_ptr() as usize; 2]),
                        layout,
                        &mut bytes
                    )),
                    0
                );
                assert_eq!(bytes, [0xa5; 16]);
            }
        }
        assert!(memory.calls.borrow().is_empty());
        let fields = [
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 0,
                len: 0,
            },
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 2,
            },
            RecordField {
                source_offset: 8,
                destination_offset: 2,
                len: 2,
            },
        ];
        let mut bytes = [0xa5; 4];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[0]),
                &RecordLayout::new(&fields, 4).expect("valid record layout"),
                &mut bytes
            ),
            1
        );
        assert!(memory.calls.borrow().is_empty());
        assert_eq!(bytes, [0xa5; 4]);
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[usize::MAX - 3]),
                &RecordLayout::new(&fields, 4).expect("valid record layout"),
                &mut bytes
            ),
            2
        );
        assert_eq!(*memory.calls.borrow(), [usize::MAX - 3]);
        assert_eq!(bytes, [0xfc, 0xfc, 0xa5, 0xa5]);
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[0, usize::MAX]),
                &RecordLayout::new(&fields[..1], 0).expect("valid record layout"),
                &mut []
            ),
            2
        );
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[]),
                &RecordLayout::new(&fields, 4).expect("valid record layout"),
                &mut []
            ),
            0
        );
    }

    #[test]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn native_records_preserve_exact_unaligned_bytes_and_fault_field_positions() {
        let fields = [
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 0,
                len: 0,
            },
            RecordField {
                source_offset: 1,
                destination_offset: 1,
                len: 1,
            },
            RecordField {
                source_offset: 3,
                destination_offset: 3,
                len: 2,
            },
            RecordField {
                source_offset: 8,
                destination_offset: 5,
                len: 12,
            },
            RecordField {
                source_offset: 24,
                destination_offset: 17,
                len: 16,
            },
            RecordField {
                source_offset: 40,
                destination_offset: 33,
                len: 8,
            },
        ];
        let source: [u8; 64] = std::array::from_fn(|index| 255 - index as u8);
        // SAFETY: readable sources are immutable Rust-owned bytes. The null,
        // overflow and inaccessible null-page sources are contained by the
        // checked C gather; Rust never dereferences these source pointers.
        let memory = unsafe { LiveMemory::new() };
        for bad_pointer in [
            0,
            usize::MAX,
            1,
            0x0072_6f74_6361_665f,
            0x0000_8000_0000_0000,
            0xffff_0000_0000_0000,
        ] {
            let mut bytes = [0xa5; 96];
            assert_eq!(
                memory.copy_records(
                    RecordSources::Pointers(&[source.as_ptr() as usize, bad_pointer]),
                    &RecordLayout::new(&fields, 48).expect("valid record layout"),
                    &mut bytes
                ),
                7
            );
            for field in &fields[1..] {
                assert_eq!(
                    bytes[field.destination_offset..field.destination_offset + field.len],
                    source[field.source_offset..field.source_offset + field.len]
                );
            }
            for index in [0, 2, 41, 47, 48] {
                assert_eq!(bytes[index], 0xa5);
            }
            // Byte 49 belongs to the failed field; no later field may change.
            assert_eq!(bytes[50..], [0xa5; 96][50..]);
        }
        let mut bytes = [0xa5; 48];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[source.as_ptr() as usize]),
                &RecordLayout::new(&fields, 48).expect("valid record layout"),
                &mut bytes
            ),
            fields.len()
        );
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[0, usize::MAX]),
                &RecordLayout::new(&fields[..1], 0).expect("valid record layout"),
                &mut []
            ),
            2
        );
    }

    #[test]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn native_records_copy_only_the_proven_prefix_including_partial_rows() {
        unsafe extern "C" {
            fn seh_guarded_copy_records(
                pointers: *const usize,
                source_base: usize,
                source_stride: usize,
                validated_field_count: usize,
                fields: *const RecordField,
                field_count: usize,
                row_stride: usize,
                destination: *mut u8,
            ) -> usize;
        }
        let source: [u8; 16] = std::array::from_fn(|index| index as u8);
        let pointers = [source.as_ptr() as usize; 2];
        let fields = [
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 0,
                len: 0,
            },
            RecordField {
                source_offset: 1,
                destination_offset: 1,
                len: 2,
            },
            RecordField {
                source_offset: 4,
                destination_offset: 4,
                len: 4,
            },
        ];
        for prefix in 0..=6 {
            let mut bytes = [0xa5; 16];
            // SAFETY: every nonempty field in this explicit prefix points to
            // immutable owned bytes; all metadata/destination bounds are valid.
            // A readable suffix intentionally remains outside the C contract.
            assert_eq!(
                unsafe {
                    seh_guarded_copy_records(
                        pointers.as_ptr(),
                        0,
                        0,
                        prefix,
                        fields.as_ptr(),
                        fields.len(),
                        8,
                        bytes.as_mut_ptr(),
                    )
                },
                prefix
            );
            let mut expected = [0xa5; 16];
            for index in 0..prefix {
                let field = fields[index % fields.len()];
                if field.len != 0 {
                    let target = index / fields.len() * 8 + field.destination_offset;
                    expected[target..target + field.len].copy_from_slice(
                        &source[field.source_offset..field.source_offset + field.len],
                    );
                }
            }
            assert_eq!(bytes, expected, "prefix={prefix}");
        }
    }

    #[test]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn native_records_unreadable_prefix_wins_over_later_preflight_failures() {
        // SAFETY: inaccessible null-page sources are only used by guarded C;
        // Rust preflight checks their arithmetic without dereferencing them.
        let memory = unsafe { LiveMemory::new() };
        for bad_offset in [usize::MAX, 0x0000_8000_0000_0000 - 1] {
            let fields = [
                RecordField {
                    source_offset: usize::MAX,
                    destination_offset: 0,
                    len: 0,
                },
                RecordField {
                    source_offset: 0,
                    destination_offset: 0,
                    len: 8,
                },
                RecordField {
                    source_offset: bad_offset,
                    destination_offset: 8,
                    len: 8,
                },
                RecordField {
                    source_offset: 0,
                    destination_offset: 16,
                    len: 8,
                },
            ];
            assert_eq!(
                record_source_prefix(
                    RecordSources::Pointers(&[1]),
                    &RecordLayout::new(&fields, usize::MAX).unwrap()
                ),
                2
            );
            let mut bytes = [0xa5; 24];
            assert_eq!(
                memory.copy_records(
                    RecordSources::Pointers(&[1]),
                    &RecordLayout::new(&fields, 24).expect("valid record layout"),
                    &mut bytes
                ),
                1
            );
            assert_eq!(bytes, [0xa5; 24]);
        }
    }

    /// A gathered value is served only when every byte of it was gathered:
    /// inside one span, never across a gap, a span edge or the map's end, and
    /// never from an incomplete record.
    #[test]
    fn gathered_values_come_only_from_covered_bytes_of_complete_records() {
        const SPANS: [(usize, usize); 3] = [(0, 8), (0x10, 4), (0x20, 16)];
        const FIELDS: [RecordField; 3] = packed_fields(SPANS);
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&FIELDS, packed_stride(SPANS)).unwrap();
        const MAP: [u16; 0x30] = packed_map(SPANS);
        let object: Vec<u8> = (0..0x30).collect();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &object,
        };
        let mut bytes = [0; 2 * 28];
        let mut complete = [false; 2];
        gather_records(&memory, &[0x1000, 0], &LAYOUT, &mut bytes, &mut complete);
        assert_eq!(complete, [true, false]);
        let record = Gathered::row(&LAYOUT, &MAP, &bytes, &complete, 0);
        let expect =
            |offset: usize| u32::from_le_bytes(object[offset..offset + 4].try_into().unwrap());
        for offset in [0, 4, 0x10, 0x20, 0x2c] {
            assert_eq!(
                record.get::<u32>(offset),
                Some(expect(offset)),
                "{offset:#x}"
            );
        }
        assert_eq!(record.get::<[u8; 16]>(0x20).unwrap(), object[0x20..0x30]);
        // Across the end of a span, a gap, an uncovered byte or the map.
        for offset in [5, 8, 0xe, 0x11, 0x1e, 0x2d, 0x30, usize::MAX] {
            assert_eq!(record.get::<u32>(offset), None, "{offset:#x}");
        }
        assert_eq!(record.get::<u64>(0x10), None);
        assert_eq!(
            Gathered::row(&LAYOUT, &MAP, &bytes, &complete, 1).get::<u32>(0),
            None
        );
        assert_eq!(Gathered::NONE.get::<u32>(0), None);
    }

    #[test]
    fn unaligned_raw_values_preserve_all_bits_and_refuse_missing_bytes() {
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &[0, 255, 255, 255, 255],
        };
        assert_eq!(read::<i32>(&memory, 0x1001), Some(-1));
        assert_eq!(read::<u32>(&memory, 0x1002), None);
        assert_eq!(read::<u64>(&memory, usize::MAX - 3), None);
        assert_eq!(read::<u32>(&memory, 0), None);
        assert_eq!(read::<[u8; 4]>(&memory, 0x1001), Some([255; 4]));
        assert_eq!(read::<[u8; 0]>(&memory, 0), Some([]));
        assert_eq!(scalar_from_bytes::<i32>(&[255; 4]), Some(-1));
        assert_eq!(scalar_from_bytes::<u32>(&[255; 3]), None);
    }

    #[test]
    fn slice_hints_accept_every_address_and_length_without_changing_reads() {
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &[1, 2, 3, 4],
        };
        for (address, len) in [
            (0, 4),
            (0xfff, 2),
            (0x1000, 0),
            (0x1002, usize::MAX),
            (0x1004, 1),
            (usize::MAX, usize::MAX),
        ] {
            memory.prefetch(address, len);
        }
        assert_eq!(read::<[u8; 4]>(&memory, 0x1000), Some([1, 2, 3, 4]));
    }

    #[test]
    fn scattered_fields_preserve_unaligned_full_width_values() {
        let mut bytes = [0; 20];
        bytes[1..3].copy_from_slice(&u16::MAX.to_le_bytes());
        bytes[5..9].copy_from_slice(&(u32::MAX - 1).to_le_bytes());
        bytes[11..19].copy_from_slice(&(i64::MIN + 0x1234).to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut narrow = [0; 2];
        let mut medium = [0; 4];
        let mut wide = [0; 8];
        assert_eq!(
            memory.copy_scattered(
                [0x1001, 0x1005, 0x100b],
                &mut [&mut narrow, &mut medium, &mut wide],
            ),
            3
        );
        assert_eq!(u16::from_le_bytes(narrow), u16::MAX);
        assert_eq!(u32::from_le_bytes(medium), u32::MAX - 1);
        assert_eq!(i64::from_le_bytes(wide), i64::MIN + 0x1234);
    }

    #[test]
    fn scattered_failure_returns_only_complete_fields_and_stops_copying() {
        let memory = TracedMemory {
            calls: RefCell::new(Vec::new()),
            fail_at: 0x1002,
        };
        let mut first = [0; 2];
        let mut failed = [0; 2];
        let mut later = [0xa5; 2];
        assert_eq!(
            memory.copy_scattered(
                [0x1001, 0x1002, 0x1003],
                &mut [&mut first, &mut failed, &mut later],
            ),
            1
        );
        assert_eq!(*memory.calls.borrow(), [0x1001, 0x1002]);
        assert_eq!(first, [1; 2]);
        assert_eq!(failed, [0x7e, 0]);
        assert_eq!(later, [0xa5; 2]);
    }

    #[test]
    fn prefetched_scattered_preserves_ordered_adapter_observations_and_failed_prefixes() {
        fn scatter(bytes: &mut [[u8; 8]; 6]) -> [&mut [u8]; 6] {
            let [first, empty, repeated, failed, invalid, later] = bytes;
            [
                first.as_mut_slice(),
                &mut empty[..0],
                repeated.as_mut_slice(),
                failed.as_mut_slice(),
                invalid.as_mut_slice(),
                later.as_mut_slice(),
            ]
        }
        for fail_at in [0, 0x1002] {
            for invalid in [0, usize::MAX, 0x1003] {
                let ordered = TracedMemory {
                    calls: RefCell::new(Vec::new()),
                    fail_at,
                };
                let prefetched = TracedMemory {
                    calls: RefCell::new(Vec::new()),
                    fail_at,
                };
                let addresses = [0x1001, 0, 0x1001, 0x1002, invalid, 0x1004];
                let mut expected = [[0xa5; 8]; 6];
                let mut actual = expected;
                let expected_count = ordered.copy_scattered(addresses, &mut scatter(&mut expected));
                let actual_count =
                    prefetched.copy_scattered_prefetched(addresses, &mut scatter(&mut actual));
                assert_eq!(actual_count, expected_count);
                assert_eq!(actual, expected);
                assert_eq!(*prefetched.calls.borrow(), *ordered.calls.borrow());
            }
        }
    }

    #[test]
    fn prefetched_scattered_uses_the_adapters_scattered_protocol() {
        struct ScatterOnly(std::cell::Cell<usize>);
        impl Memory for ScatterOnly {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("the adapter's scattered path must own all observations");
            }
            fn copy_scattered<const N: usize>(
                &self,
                _: [usize; N],
                destinations: &mut [&mut [u8]; N],
            ) -> usize {
                self.0.set(self.0.get() + 1);
                for destination in destinations {
                    destination.fill(0x6d);
                }
                N
            }
        }
        let memory = ScatterOnly(std::cell::Cell::new(0));
        let mut first = [0; 8];
        let mut second = [0; 4];
        assert_eq!(
            memory.copy_scattered_prefetched([0x1000, 0x2000], &mut [&mut first, &mut second]),
            2
        );
        assert_eq!(first, [0x6d; 8]);
        assert_eq!(second, [0x6d; 4]);
        assert_eq!(memory.0.get(), 1);
    }

    #[test]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn native_prefetched_scattered_keeps_empty_repeated_and_invalid_prefixes() {
        let source: [u8; 16] = std::array::from_fn(|index| index as u8);
        // SAFETY: valid sources are immutable owned bytes; invalid sources are
        // rejected by preflight or contained in the native exact-source guard.
        let memory = unsafe { LiveMemory::new() };
        for invalid in [0, usize::MAX, 1, 0x0000_8000_0000_0000] {
            let mut empty = [];
            let mut first = [0xa5; 8];
            let mut repeated = [0xa5; 8];
            let mut failed = [0xa5; 8];
            let mut later = [0xa5; 8];
            assert_eq!(
                memory.copy_scattered_prefetched(
                    [
                        0,
                        source.as_ptr() as usize + 1,
                        source.as_ptr() as usize + 1,
                        invalid,
                        source.as_ptr() as usize
                    ],
                    &mut [
                        &mut empty,
                        &mut first,
                        &mut repeated,
                        &mut failed,
                        &mut later
                    ],
                ),
                3,
            );
            assert_eq!(first, source[1..9]);
            assert_eq!(repeated, first);
            assert_eq!(later, [0xa5; 8]);
        }
        let mut first_empty = [];
        let mut second_empty = [];
        assert_eq!(
            memory.copy_scattered_prefetched(
                [0, usize::MAX],
                &mut [&mut first_empty, &mut second_empty]
            ),
            2
        );
        assert_eq!(memory.copy_scattered_prefetched([], &mut []), 0);
    }

    #[test]
    fn scattered_invalid_addresses_leave_the_valid_prefix_copied() {
        for invalid in [0, usize::MAX - 1] {
            let memory = TracedMemory {
                calls: RefCell::new(Vec::new()),
                fail_at: usize::MAX,
            };
            let mut first = [0; 2];
            let mut failed = [0xa5; 2];
            let mut later = [0xa5; 2];
            assert_eq!(
                memory.copy_scattered(
                    [0x1001, invalid, 0x1003],
                    &mut [&mut first, &mut failed, &mut later],
                ),
                1
            );
            assert_eq!(*memory.calls.borrow(), [0x1001]);
            assert_eq!(first, [1; 2]);
            assert_eq!(failed, [0xa5; 2]);
            assert_eq!(later, [0xa5; 2]);
        }
    }

    #[test]
    fn scattered_empty_fields_succeed_without_copying_or_validating_addresses() {
        let memory = TracedMemory {
            calls: RefCell::new(Vec::new()),
            fail_at: usize::MAX,
        };
        let mut first_empty = [];
        let mut second_empty = [];
        let mut value = [0; 2];
        assert_eq!(
            memory.copy_scattered(
                [0, usize::MAX, 0x1001],
                &mut [&mut first_empty, &mut second_empty, &mut value],
            ),
            3
        );
        assert_eq!(*memory.calls.borrow(), [0x1001]);
        assert_eq!(value, [1; 2]);
        assert_eq!(memory.copy_scattered([], &mut []), 0);
    }

    #[test]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn native_scattered_copy_preserves_the_prefix_before_an_invalid_address() {
        let source = (i64::MIN + 0x1234).to_le_bytes();
        // SAFETY: the only accessed source is immutable, Rust-owned memory that
        // outlives the call. The zero address is rejected before entering C.
        let memory = unsafe { LiveMemory::new() };
        let mut empty = [];
        let mut first = [0; 8];
        let mut failed = [0xa5; 4];
        let mut later = [0xa5; 8];
        assert_eq!(
            memory.copy_scattered(
                [0, source.as_ptr() as usize, 0, source.as_ptr() as usize],
                &mut [&mut empty, &mut first, &mut failed, &mut later],
            ),
            2
        );
        assert_eq!(first, source);
        assert_eq!(failed, [0xa5; 4]);
        assert_eq!(later, [0xa5; 8]);
        let mut second_empty = [];
        assert_eq!(
            memory.copy_scattered(
                [source.as_ptr() as usize, 0, usize::MAX],
                &mut [&mut first, &mut empty, &mut second_empty],
            ),
            3
        );
        assert_eq!(first, source);
        assert_eq!(
            memory.copy_scattered([0, usize::MAX], &mut [&mut empty, &mut second_empty]),
            2
        );
    }

    #[test]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn native_scattered_fault_preserves_completed_fields_and_leaves_later_fields_alone() {
        let source: [u8; 65] = std::array::from_fn(|index| index as u8);
        let source = &source[1..];
        // SAFETY: valid sources are immutable owned bytes. The intentionally
        // inaccessible Windows null-page address faults inside the C SEH shim;
        // it is never dereferenced by Rust or returned as a Rust reference.
        let memory = unsafe { LiveMemory::new() };
        for width in [1, 2, 4, 8, 12, 16, 24, 26, 32, 48, 64] {
            let mut first = [0xa5; 64];
            let mut empty = [];
            let mut failed = [0xa5; 64];
            let mut later = [0xa5; 8];
            assert_eq!(
                memory.copy_scattered(
                    [source.as_ptr() as usize, 0, 1, source.as_ptr() as usize],
                    &mut [
                        &mut first[..width],
                        &mut empty,
                        &mut failed[..width],
                        &mut later,
                    ],
                ),
                2
            );
            assert_eq!(first[..width], source[..width]);
            assert_eq!(first[width..], [0xa5; 64][width..]);
            assert_eq!(later, [0xa5; 8]);
        }
    }

    /// `copy_uninit` returns exactly the bytes `copy` writes, and nothing
    /// when the copy fails, in every source: the guarded live copy across a
    /// page it cannot read (where a failed copy may already have written a
    /// prefix), the owned slice, the counting adapter, and the default that
    /// initializes first. `read_slice_uninit` keeps `read_slice`'s gates.
    #[test]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn uninitialized_copies_expose_only_complete_copies() {
        use std::{ffi::c_void, mem::MaybeUninit};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn VirtualAlloc(
                address: *mut c_void,
                size: usize,
                kind: u32,
                protect: u32,
            ) -> *mut c_void;
            fn VirtualProtect(
                address: *mut c_void,
                size: usize,
                protect: u32,
                old: *mut u32,
            ) -> i32;
            fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
        }
        struct Pages(*mut c_void);
        impl Drop for Pages {
            fn drop(&mut self) {
                // SAFETY: this test owns the complete allocation until here.
                unsafe { VirtualFree(self.0, 0, 0x8000) };
            }
        }
        // SAFETY: allocation has no requested address and no executable pages.
        let pages = Pages(unsafe { VirtualAlloc(std::ptr::null_mut(), 8192, 0x3000, 4) });
        assert!(!pages.0.is_null());
        let boundary = pages.0 as usize + 4096;
        let mut old = 0;
        // SAFETY: both pages belong to this allocation; only page two is protected.
        assert_ne!(
            unsafe { VirtualProtect(boundary as *mut c_void, 4096, 1, &mut old) },
            0
        );
        // SAFETY: the first page remains owned and writable.
        let first = unsafe { std::slice::from_raw_parts_mut(pages.0.cast::<u8>(), 4096) };
        for (index, byte) in first.iter_mut().enumerate() {
            *byte = index as u8 ^ 0x5a;
        }
        // SAFETY: all sources are this test's pages; faults are contained.
        let live = unsafe { LiveMemory::new() };

        /// Default adapter: only `copy`, so `copy_uninit` initializes first.
        struct Plain<'a>(&'a LiveMemory);
        impl Memory for Plain<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.0.copy(address, destination)
            }
        }
        let counted = CountedMemory::new(unsafe { LiveMemory::new() });
        let slice = SliceMemory {
            base: pages.0 as usize,
            bytes: &first[..],
        };
        fn check(memory: &impl Memory, boundary: usize) {
            for width in [1, 4, 8, 13, 64, 256, 512] {
                for (address, whole) in [
                    (boundary - width, true),
                    (boundary - width + 1, false),
                    (0, false),
                    (usize::MAX - 2, false),
                ] {
                    let mut expected = vec![0xa5; width];
                    let copied = memory.copy(address, &mut expected);
                    assert_eq!(copied, whole, "{address:#x}+{width}");
                    let mut scratch = [MaybeUninit::<u8>::uninit(); 512];
                    match memory.copy_uninit(address, &mut scratch[..width]) {
                        Some(bytes) => {
                            assert!(whole);
                            assert_eq!(&bytes[..], &expected[..]);
                        }
                        None => assert!(!whole),
                    }
                    let mut scratch = [MaybeUninit::<u8>::uninit(); 512];
                    let mut plain = vec![0; width];
                    assert_eq!(
                        read_slice_uninit(memory, address, &mut scratch[..width])
                            .map(|bytes| bytes.to_vec()),
                        read_slice(memory, address, &mut plain).then_some(plain)
                    );
                }
            }
            let mut empty: [MaybeUninit<u8>; 0] = [];
            assert_eq!(
                read_slice_uninit(memory, 0, &mut empty).map(|b| b.len()),
                Some(0)
            );
        }
        check(&live, boundary);
        check(&Plain(&live), boundary);
        check(&counted, boundary);
        check(&slice, boundary);
        // The counting adapter counts an uninitialized copy as the copy it is.
        ReadCounts::take_current();
        let mut scratch = [MaybeUninit::<u8>::uninit(); 16];
        assert!(counted.copy_uninit(boundary - 16, &mut scratch).is_some());
        let uninit = ReadCounts::take_current();
        assert!(counted.copy(boundary - 16, &mut [0; 16]));
        assert_eq!(uninit, ReadCounts::take_current());
        assert_eq!((uninit.copy_calls, uninit.spans, uninit.bytes), (1, 1, 16));
    }

    #[test]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn native_copy_widths_stop_at_exact_page_boundaries_and_keep_field_prefixes() {
        use std::ffi::c_void;
        unsafe extern "C" {
            fn seh_guarded_copy(dst: *mut u8, src: *const u8, len: usize) -> i32;
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn VirtualAlloc(
                address: *mut c_void,
                size: usize,
                kind: u32,
                protect: u32,
            ) -> *mut c_void;
            fn VirtualProtect(
                address: *mut c_void,
                size: usize,
                protect: u32,
                old: *mut u32,
            ) -> i32;
            fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
        }
        struct Pages(*mut c_void);
        impl Drop for Pages {
            fn drop(&mut self) {
                // SAFETY: this test owns the complete allocation until here.
                unsafe { VirtualFree(self.0, 0, 0x8000) };
            }
        }
        // Two owned Windows pages: readable bytes followed by an inaccessible
        // page. No game process or external address space is involved.
        // SAFETY: allocation has no requested address and no executable pages.
        let pages = Pages(unsafe { VirtualAlloc(std::ptr::null_mut(), 8192, 0x3000, 4) });
        assert!(!pages.0.is_null());
        let boundary = pages.0 as usize + 4096;
        let mut old = 0;
        // SAFETY: both pages belong to this allocation; only page two is protected.
        assert_ne!(
            unsafe { VirtualProtect(boundary as *mut c_void, 4096, 1, &mut old) },
            0
        );
        // SAFETY: the first page remains owned and writable.
        unsafe { std::ptr::write_bytes(pages.0.cast::<u8>(), 0x6d, 4096) };
        // SAFETY: all inputs below are owned storage or the test's inaccessible
        // page, whose faults are contained by the same native guarded copies.
        let memory = unsafe { LiveMemory::new() };
        let zero = [0_u8; 5];
        let all_bits = [255_u8; 5];
        assert_eq!(read::<u32>(&memory, zero[1..].as_ptr() as usize), Some(0));
        assert_eq!(
            read::<i32>(&memory, all_bits[1..].as_ptr() as usize),
            Some(-1)
        );
        for width in [1, 2, 4, 5, 8, 12, 13, 16, 24, 26, 32, 48, 64] {
            let valid = boundary - width;
            let crossing = valid + 1;
            let mut exact = [0xa5; 66];
            assert!(memory.copy(valid, &mut exact[1..1 + width]));
            assert_eq!(exact[1..1 + width], [0x6d; 64][..width]);
            // The fixed-width entries also accept an unaligned source; the
            // destination uses the same one-byte inset for both successful reads.
            assert!(memory.copy(valid - 1, &mut exact[1..1 + width]));
            assert_eq!(exact[1..1 + width], [0x6d; 64][..width]);
            assert_eq!(exact[0], 0xa5);
            assert!(exact[1 + width..].iter().all(|&byte| byte == 0xa5));
            assert!(!memory.copy(crossing, &mut exact[1..1 + width]));
            assert_eq!(exact[0], 0xa5);
            assert!(exact[1 + width..].iter().all(|&byte| byte == 0xa5));
            if matches!(width, 4 | 8) {
                // Direct generic FFI callers still accept these widths through
                // memcpy after LiveMemory selects the dedicated scalar entries.
                for (address, expected) in [(valid, 1), (valid - 1, 1), (crossing, 0)] {
                    exact.fill(0xa5);
                    // SAFETY: destination is owned writable storage; all source
                    // bytes or faults belong to this test's guarded allocation.
                    let result = unsafe {
                        seh_guarded_copy(exact[1..].as_mut_ptr(), address as *const u8, width)
                    };
                    assert_eq!(result, expected);
                    if expected != 0 {
                        assert_eq!(exact[1..1 + width], [0x6d; 64][..width]);
                    }
                    assert_eq!(exact[0], 0xa5);
                    assert!(exact[1 + width..].iter().all(|&byte| byte == 0xa5));
                }
            }
            if width == 4 {
                for address in [valid, valid - 1] {
                    assert_eq!(read::<u32>(&memory, address), Some(0x6d6d_6d6d));
                    assert_eq!(read::<i32>(&memory, address), Some(0x6d6d_6d6d));
                    assert_eq!(read::<[u8; 4]>(&memory, address), Some([0x6d; 4]));
                }
                assert_eq!(read::<u32>(&memory, crossing), None);
                assert_eq!(read::<i32>(&memory, crossing), None);
                assert_eq!(read::<[u8; 4]>(&memory, crossing), None);
            }

            let mut first = [0xa5; 64];
            let mut failed = [0xa5; 64];
            let mut later = [0xa5; 64];
            assert_eq!(
                memory.copy_scattered(
                    [valid, crossing, valid],
                    &mut [
                        &mut first[..width],
                        &mut failed[..width],
                        &mut later[..width]
                    ],
                ),
                1
            );
            assert_eq!(first[..width], [0x6d; 64][..width]);
            assert_eq!(later, [0xa5; 64]);
            let field = [RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: width,
            }];
            let mut records = [0xa5; 192];
            assert_eq!(
                memory.copy_records(
                    RecordSources::Pointers(&[valid, crossing, valid]),
                    &RecordLayout::new(&field, width).expect("valid record layout"),
                    &mut records
                ),
                1
            );
            assert_eq!(records[..width], [0x6d; 64][..width]);
            assert!(records[width * 2..].iter().all(|&byte| byte == 0xa5));
        }
        // A guard-page exception is the first failed scalar observation too;
        // there is no retry after the operating system consumes that guard.
        // SAFETY: page two belongs to this test and no Rust reference covers it.
        assert_ne!(
            unsafe { VirtualProtect(boundary as *mut c_void, 4096, 0x104, &mut old) },
            0
        );
        assert_eq!(read::<u32>(&memory, boundary - 3), None);
    }

    #[test]
    fn canonical_range_rejects_live_crash_vectors_and_keeps_valid_spans() {
        // Owner-measured crash sources: ASCII "_factor\0" and "ts_facto" bytes
        // read as pointers. Both are noncanonical (top bits nonzero while bit
        // 47 is zero) and must fail pre-copy with the same outcome as an
        // unreadable canonical source.
        assert!(!is_canonical_range(0x0072_6f74_6361_665f, 8));
        assert!(!is_canonical_range(0x6f74_6361_665f_7374, 8));
        assert!(!is_canonical_range(0x0000_8000_0000_0000, 8));
        assert!(!is_canonical_range(0xFFFF_0000_0000_0000, 8));
        assert!(is_canonical_range(0x1_4000_0000, 8));
        assert!(is_canonical_range(0xFFFF_8000_0000_0000, 8));
        assert!(!is_canonical_range(0x0000_7FFF_FFFF_FFF0, 32));
        assert!(is_canonical_range(0x2000, 0));
        assert!(!is_canonical_range(usize::MAX - 3, 8));
    }

    #[test]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn records_gate_derived_noncanonical_fields_in_copy_order() {
        let source = 0x1234_5678_9abc_def0_u64.to_le_bytes();
        let pointer = source.as_ptr() as usize;
        let fields = [
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 0,
                len: 0,
            },
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 8,
            },
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 8,
                len: 0,
            },
            RecordField {
                source_offset: 0x0000_8000_0000_0000 - pointer,
                destination_offset: 8,
                len: 8,
            },
            RecordField {
                source_offset: 0,
                destination_offset: 16,
                len: 8,
            },
        ];
        // SAFETY: the only readable source is immutable Rust-owned storage;
        // The gather rejects the derived noncanonical address before reading it.
        let memory = unsafe { LiveMemory::new() };
        let mut bytes = [0xa5; 24];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[pointer]),
                &RecordLayout::new(&fields, 24).expect("valid record layout"),
                &mut bytes
            ),
            3
        );
        assert_eq!(bytes[..8], source);
        assert_eq!(bytes[8..], [0xa5; 16]);
        // An earlier unreadable canonical field wins over the later invalid
        // derived field. Empty fields count but do not inspect their source.
        bytes.fill(0xa5);
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[1]),
                &RecordLayout::new(&fields, 24).expect("valid record layout"),
                &mut bytes
            ),
            1
        );
        assert_eq!(bytes, [0xa5; 24]);
        // A range crossing the canonical gap must fail before copying any of
        // it, even though its starting address is itself canonical.
        let crossing = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 24,
        }];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[0x0000_7fff_ffff_fff0]),
                &RecordLayout::new(&crossing, 24).expect("valid record layout"),
                &mut bytes
            ),
            0
        );
        assert_eq!(bytes, [0xa5; 24]);
    }

    #[test]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn live_memory_rejects_noncanonical_sources_pre_copy() {
        // SAFETY: noncanonical sources never reach the guarded instruction;
        // the Rust gate returns `false`/prefix without dereferencing them.
        // Canonical valid sources still copy; canonical unreadable (null page)
        // still fails gracefully via the C shim.
        let memory = unsafe { LiveMemory::new() };
        let mut out = [0xa5; 8];
        assert!(!memory.copy(0x0072_6f74_6361_665f, &mut out));
        assert!(!memory.copy(0x6f74_6361_665f_7374, &mut out));
        assert_eq!(out, [0xa5; 8]);
        let source = 0x1234_5678_u64.to_le_bytes();
        let mut valid = [0; 8];
        assert!(memory.copy(source.as_ptr() as usize, &mut valid));
        assert_eq!(valid, source);
        // Scattered prefix stops at the noncanonical span with earlier fields
        // intact, exactly like a null/overflow break.
        let mut first = [0; 8];
        let mut failed = [0xa5; 8];
        let mut later = [0xa5; 8];
        assert_eq!(
            memory.copy_scattered(
                [
                    source.as_ptr() as usize,
                    0x0072_6f74_6361_665f,
                    source.as_ptr() as usize
                ],
                &mut [&mut first, &mut failed, &mut later],
            ),
            1
        );
        assert_eq!(first, source);
        assert_eq!(failed, [0xa5; 8]);
        assert_eq!(later, [0xa5; 8]);
        // Records prefix stops at the noncanonical field just as it does for
        // null/overflow, with the leading row's bytes preserved.
        let fields = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        }];
        let mut bytes = [0xa5; 16];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[source.as_ptr() as usize, 0x0072_6f74_6361_665f]),
                &RecordLayout::new(&fields[..1], 8).expect("valid record layout"),
                &mut bytes
            ),
            1
        );
        assert_eq!(&bytes[..8], &source);
        assert_eq!(&bytes[8..], &[0xa5; 8]);
    }

    #[test]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn guarded_copy_handles_noncanonical_fault_address_gracefully() {
        // Direct C containment proof for `copy_fault`'s `-1` path, bypassing
        // the Rust gate. Before the fix this crashed with
        // EXCEPTION_CONTINUE_SEARCH; after the fix it returns graceful
        // failure (0). The Rust gate makes this path unreachable through
        // `LiveMemory`/`take_inner`, so this test is the only caller that
        // exercises the C `-1` branch directly.
        unsafe extern "C" {
            fn seh_guarded_copy(dst: *mut u8, src: *const u8, len: usize) -> i32;
            fn seh_guarded_copy4(dst: *mut u8, src: *const u8) -> i32;
            fn seh_guarded_copy8(dst: *mut u8, src: *const u8) -> i32;
            fn seh_guarded_read4(src: *const u8) -> u64;
        }
        let mut dst = [0xa5; 8];
        // SAFETY: destination is valid stack memory; the noncanonical source
        // is only touched by the guarded C copy, which handles the fault.
        let result =
            unsafe { seh_guarded_copy(dst.as_mut_ptr(), 0x0072_6f74_6361_665f as *const u8, 8) };
        assert_eq!(result, 0);
        assert_eq!(dst, [0xa5; 8]);
        for copy in [seh_guarded_copy4, seh_guarded_copy8] {
            // SAFETY: each entry writes at most eight valid destination bytes;
            // its identical SEH filter contains the noncanonical source fault.
            let result = unsafe { copy(dst.as_mut_ptr(), 0x0072_6f74_6361_665f as *const u8) };
            assert_eq!(result, 0);
            assert_eq!(dst, [0xa5; 8]);
        }
        // SAFETY: the value-return entry retains identical noncanonical source
        // containment, without exposing any failed partial value.
        assert_eq!(
            unsafe { seh_guarded_read4(0x0072_6f74_6361_665f as *const u8) },
            0
        );
    }

    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    mod native_record_lookahead {
        use super::*;
        use std::ffi::c_void;

        const PAGE: usize = 4096;
        const READWRITE: u32 = 4;
        const NOACCESS: u32 = 1;
        const GUARD: u32 = 0x100;

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn VirtualAlloc(
                address: *mut c_void,
                size: usize,
                kind: u32,
                protect: u32,
            ) -> *mut c_void;
            fn VirtualProtect(
                address: *mut c_void,
                size: usize,
                protect: u32,
                old: *mut u32,
            ) -> i32;
            fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
        }

        unsafe extern "C" {
            fn seh_guarded_copy_records(
                pointers: *const usize,
                source_base: usize,
                source_stride: usize,
                validated_field_count: usize,
                fields: *const RecordField,
                field_count: usize,
                row_stride: usize,
                destination: *mut u8,
            ) -> usize;
        }

        struct Pages {
            address: *mut c_void,
            count: usize,
        }

        impl Pages {
            fn new(count: usize) -> Self {
                // SAFETY: owned, writable, non-executable synthetic storage.
                let address =
                    unsafe { VirtualAlloc(std::ptr::null_mut(), count * PAGE, 0x3000, READWRITE) };
                assert!(!address.is_null());
                let pages = Self { address, count };
                for page in 0..count {
                    // SAFETY: every allocated page is still writable here.
                    unsafe {
                        std::ptr::write_bytes(pages.at(page) as *mut u8, 0x20 + page as u8, PAGE)
                    };
                }
                pages
            }

            fn at(&self, page: usize) -> usize {
                assert!(page < self.count);
                self.address as usize + page * PAGE
            }

            fn protect(&self, page: usize, protection: u32) -> u32 {
                let mut previous = 0;
                // SAFETY: changes only one page of this test's own allocation.
                assert_ne!(
                    unsafe {
                        VirtualProtect(
                            self.at(page) as *mut c_void,
                            PAGE,
                            protection,
                            &mut previous,
                        )
                    },
                    0
                );
                previous
            }
        }

        impl Drop for Pages {
            fn drop(&mut self) {
                // SAFETY: the entire allocation belongs to this fixture.
                unsafe { VirtualFree(self.address, 0, 0x8000) };
            }
        }

        #[test]
        fn prefetched_scattered_matches_native_faults_at_every_span_and_page_boundary() {
            const ROWS: usize = 9;
            let pages = Pages::new(ROWS * 2);
            // SAFETY: the fixture owns all readable and deliberately protected
            // source pages. Only the native guarded copy may dereference them.
            let memory = unsafe { LiveMemory::new() };
            for width in [1, 2, 4, 8, 12, 16, 24, 26, 32, 48, 64] {
                for offset in [0, 61, PAGE - 4] {
                    let addresses =
                        std::array::from_fn::<_, ROWS, _>(|row| pages.at(row * 2) + offset);
                    for failed in 0..ROWS {
                        let failed_page = failed * 2 + usize::from(offset + width > PAGE);
                        for protection in [NOACCESS, READWRITE | GUARD] {
                            pages.protect(failed_page, protection);
                            let mut expected = [[0xa5; 64]; ROWS];
                            let count = memory.copy_scattered(
                                addresses,
                                &mut expected.each_mut().map(|bytes| &mut bytes[..width]),
                            );
                            assert_eq!(count, failed);
                            pages.protect(failed_page, protection);
                            let mut actual = [[0xa5; 64]; ROWS];
                            let count = memory.copy_scattered_prefetched(
                                addresses,
                                &mut actual.each_mut().map(|bytes| &mut bytes[..width]),
                            );
                            assert_eq!(count, failed);
                            // Failed spans may be partial in either implementation.
                            // Every complete prefix, padding byte and later span
                            // must still be identical to the existing native path.
                            actual[failed][..width].fill(0xa5);
                            expected[failed][..width].fill(0xa5);
                            assert_eq!(actual, expected);
                            assert!(
                                actual[failed + 1..]
                                    .iter()
                                    .flatten()
                                    .all(|&byte| byte == 0xa5)
                            );
                            let observed = pages.protect(failed_page, READWRITE);
                            assert_eq!(
                                observed,
                                if protection & GUARD != 0 {
                                    READWRITE
                                } else {
                                    NOACCESS
                                }
                            );
                        }
                    }
                }
            }
        }

        #[test]
        fn prefetched_scattered_future_hints_leave_guard_pages_armed() {
            let pages = Pages::new(9);
            let addresses = std::array::from_fn::<_, 9, _>(|row| pages.at(row) + 61);
            // SAFETY: all sources belong to this synthetic fixture. Its page
            // protections exercise the native guard without accessing the game.
            let memory = unsafe { LiveMemory::new() };
            for (failed, guarded) in [(0, 1), (0, 4), (1, 5), (4, 8)] {
                pages.protect(failed, NOACCESS);
                pages.protect(guarded, READWRITE | GUARD);
                let mut output = [[0xa5; 8]; 9];
                assert_eq!(
                    memory.copy_scattered_prefetched(
                        addresses,
                        &mut output.each_mut().map(|bytes| bytes.as_mut_slice())
                    ),
                    failed,
                );
                for (row, bytes) in output[..failed].iter().enumerate() {
                    assert_eq!(*bytes, [0x20 + row as u8; 8]);
                }
                assert!(output[failed..].iter().flatten().all(|&byte| byte == 0xa5));
                assert_eq!(pages.protect(failed, READWRITE), NOACCESS);
                assert_eq!(pages.protect(guarded, READWRITE), READWRITE | GUARD);
            }
        }

        #[test]
        fn strided_native_faults_keep_exact_fields_and_overflow_prefixes() {
            const FIELDS: [RecordField; 3] = [
                RecordField {
                    source_offset: usize::MAX,
                    destination_offset: 0,
                    len: 0,
                },
                RecordField {
                    source_offset: PAGE - 4,
                    destination_offset: 1,
                    len: 8,
                },
                RecordField {
                    source_offset: 2 * PAGE,
                    destination_offset: 12,
                    len: 4,
                },
            ];
            const LAYOUT: RecordLayout<'static> = RecordLayout::new(&FIELDS, 16).unwrap();
            let pages = Pages::new(9);
            // SAFETY: this fixture owns every source page and destination.
            let memory = unsafe { LiveMemory::new() };
            for failed_row in 0..3 {
                for (failed_field, failed) in FIELDS.iter().enumerate().skip(1) {
                    for protection in [NOACCESS, READWRITE | GUARD] {
                        let failed_page = failed_row * 3 + failed_field;
                        pages.protect(failed_page, protection);
                        let mut output = [0xa5; 48];
                        let copied = memory.copy_records(
                            RecordSources::Strided {
                                base: pages.at(0),
                                stride: 3 * PAGE,
                                count: 3,
                            },
                            &LAYOUT,
                            &mut output,
                        );
                        assert_eq!(copied, failed_row * 3 + failed_field);
                        let mut expected = [0xa5; 48];
                        for index in 0..copied {
                            let row = index / FIELDS.len();
                            let field = FIELDS[index % FIELDS.len()];
                            for offset in 0..field.len {
                                expected[row * 16 + field.destination_offset + offset] =
                                    0x20 + (row * 3 + (field.source_offset + offset) / PAGE) as u8;
                            }
                        }
                        // The failed field may be partially written. Its exact
                        // completed prefix and every later field remain fixed.
                        let start = failed_row * 16 + failed.destination_offset;
                        output[start..start + failed.len].fill(0xa5);
                        assert_eq!(output, expected);
                        pages.protect(failed_page, READWRITE);
                    }
                }
            }
            const EMPTY_FIRST: [RecordField; 3] = [
                RecordField {
                    source_offset: usize::MAX,
                    destination_offset: 0,
                    len: 0,
                },
                RecordField {
                    source_offset: 0,
                    destination_offset: 0,
                    len: 8,
                },
                RecordField {
                    source_offset: usize::MAX,
                    destination_offset: 8,
                    len: 0,
                },
            ];
            const OVERFLOW_LAYOUT: RecordLayout<'static> =
                RecordLayout::new(&EMPTY_FIRST, 8).unwrap();
            let sources = RecordSources::Strided {
                base: pages.at(0),
                stride: usize::MAX - pages.at(0) + 1,
                count: 2,
            };
            let mut output = [0xa5; 16];
            assert_eq!(record_source_prefix(sources, &OVERFLOW_LAYOUT), 4);
            assert_eq!(
                memory.copy_records(sources, &OVERFLOW_LAYOUT, &mut output),
                4
            );
            assert_eq!(&output[..8], &[0x20; 8]);
            assert_eq!(&output[8..], &[0xa5; 8]);
            pages.protect(0, NOACCESS);
            output.fill(0xa5);
            assert_eq!(
                memory.copy_records(sources, &OVERFLOW_LAYOUT, &mut output),
                1
            );
            assert_eq!(output, [0xa5; 16]);
        }

        fn gather(
            pointers: &[usize],
            fields: &[RecordField],
            prefix: usize,
            stride: usize,
            destination: &mut [u8],
        ) -> usize {
            assert!(
                prefix
                    <= RecordLayout::new(fields, stride)
                        .and_then(|layout| layout
                            .count(RecordSources::Pointers(pointers), destination.len()))
                        .unwrap()
            );
            assert!(
                prefix
                    <= record_source_prefix(
                        RecordSources::Pointers(pointers),
                        &RecordLayout::new(fields, usize::MAX).unwrap()
                    )
            );
            // SAFETY: the checks above establish the production native metadata
            // contract. Sources are owned readable or deliberately protected
            // test pages, which only the native guard may actually read.
            unsafe {
                seh_guarded_copy_records(
                    pointers.as_ptr(),
                    0,
                    0,
                    prefix,
                    fields.as_ptr(),
                    fields.len(),
                    stride,
                    destination.as_mut_ptr(),
                )
            }
        }

        #[test]
        fn every_owned_page_fault_keeps_every_flattened_prefix_exact() {
            const ROWS: usize = 9;
            const STRIDE: usize = 64;
            let pages = Pages::new(ROWS * 3);
            let pointers: [usize; ROWS] = std::array::from_fn(|row| pages.at(row * 3));
            let fields = [
                RecordField {
                    source_offset: usize::MAX,
                    destination_offset: 0,
                    len: 0,
                },
                RecordField {
                    source_offset: 61,
                    destination_offset: 1,
                    len: 8,
                },
                RecordField {
                    source_offset: PAGE + 61,
                    destination_offset: 12,
                    len: 12,
                },
                RecordField {
                    source_offset: 3 * PAGE - 24,
                    destination_offset: 28,
                    len: 24,
                },
            ];
            for failed_row in 0..ROWS {
                for failed_field in 1..fields.len() {
                    let failed_index = failed_row * fields.len() + failed_field;
                    let failed_page = failed_row * 3 + failed_field - 1;
                    for protection in [NOACCESS, READWRITE | GUARD] {
                        for prefix in 0..=ROWS * fields.len() {
                            pages.protect(failed_page, protection);
                            let mut output = [0xa5; ROWS * STRIDE];
                            let expected_prefix = prefix.min(failed_index);
                            assert_eq!(
                                gather(&pointers, &fields, prefix, STRIDE, &mut output),
                                expected_prefix,
                                "row={failed_row} field={failed_field} prefix={prefix}"
                            );
                            let mut expected = [0xa5; ROWS * STRIDE];
                            for index in 0..expected_prefix {
                                let row = index / fields.len();
                                let field = fields[index % fields.len()];
                                if field.len != 0 {
                                    let value = 0x20 + (row * 3 + field.source_offset / PAGE) as u8;
                                    let start = row * STRIDE + field.destination_offset;
                                    expected[start..start + field.len].fill(value);
                                }
                            }
                            assert_eq!(
                                output, expected,
                                "row={failed_row} field={failed_field} prefix={prefix}"
                            );
                            // VirtualProtect returns the old protection without
                            // touching source bytes. A future hinted guard page
                            // must remain armed until its actual guarded read.
                            let observed = pages.protect(failed_page, READWRITE);
                            let expected_protection =
                                if protection & GUARD != 0 && prefix > failed_index {
                                    READWRITE
                                } else {
                                    protection
                                };
                            assert_eq!(observed, expected_protection);
                        }
                    }
                }
            }
        }

        #[test]
        fn future_guard_pages_survive_an_earlier_noaccess_failure() {
            let pages = Pages::new(9);
            let pointers: [usize; 9] = std::array::from_fn(|row| pages.at(row));
            let fields = [RecordField {
                source_offset: 61,
                destination_offset: 0,
                len: 8,
            }];
            // Includes initial-window hints and a future row reached only by
            // the rolling four-row lookahead after earlier rows were copied.
            for (failed_row, guarded_row) in [(0, 1), (0, 4), (1, 5), (4, 8)] {
                pages.protect(failed_row, NOACCESS);
                pages.protect(guarded_row, READWRITE | GUARD);
                let mut output = [0xa5; 9 * 8];
                assert_eq!(gather(&pointers, &fields, 9, 8, &mut output), failed_row);
                for row in 0..failed_row {
                    assert_eq!(&output[row * 8..row * 8 + 8], &[0x20 + row as u8; 8]);
                }
                assert!(output[failed_row * 8..].iter().all(|&byte| byte == 0xa5));
                assert_eq!(pages.protect(failed_row, READWRITE), NOACCESS);
                assert_eq!(
                    pages.protect(guarded_row, READWRITE),
                    READWRITE | GUARD,
                    "a hint must not consume the future guard"
                );
                // Confirm that this is a real guard: an actual guarded copy
                // fails once and consumes it, rather than accepting cached data.
                pages.protect(guarded_row, READWRITE | GUARD);
                // SAFETY: only the native guard reads this owned protected page.
                let memory = unsafe { LiveMemory::new() };
                let mut actual = [0xa5; 8];
                assert!(!memory.copy(pointers[guarded_row] + 61, &mut actual));
                assert_eq!(actual, [0xa5; 8]);
                assert_eq!(pages.protect(guarded_row, READWRITE), READWRITE);
            }
        }

        const POINTER_FIELDS: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        }];

        #[test]
        fn pointer_layout_keeps_owned_faults_and_every_prefix_exact() {
            const ROWS: usize = 9;
            let pages = Pages::new(ROWS * 2);
            // All layouts match the native dispatcher. Moving the owned base
            // pointer covers aligned, cache-line and page-crossing reads while
            // keeping source_offset zero and each observation exactly 8 bytes.
            for offset in [0, 61, PAGE - 4] {
                let pointers: [usize; ROWS] = std::array::from_fn(|row| pages.at(row * 2) + offset);
                for failed_row in 0..ROWS {
                    let failed_page = failed_row * 2 + usize::from(offset + 8 > PAGE);
                    for protection in [NOACCESS, READWRITE | GUARD] {
                        for prefix in 0..=ROWS {
                            pages.protect(failed_page, protection);
                            let mut output = [0xa5; ROWS * 8];
                            let expected_prefix = prefix.min(failed_row);
                            assert_eq!(
                                gather(&pointers, &POINTER_FIELDS, prefix, 8, &mut output),
                                expected_prefix,
                                "offset={offset} failed_row={failed_row} prefix={prefix}"
                            );
                            let mut expected = [0xa5; ROWS * 8];
                            for row in 0..expected_prefix {
                                for byte in 0..8 {
                                    expected[row * 8 + byte] =
                                        0x20 + (row * 2 + (offset + byte) / PAGE) as u8;
                                }
                            }
                            assert_eq!(output, expected);
                            let expected_protection =
                                if protection & GUARD != 0 && prefix > failed_row {
                                    READWRITE
                                } else {
                                    protection
                                };
                            assert_eq!(
                                pages.protect(failed_page, READWRITE),
                                expected_protection,
                                "a hint must not consume the failing row's guard"
                            );
                        }
                    }
                }
            }
        }

        #[test]
        fn pointer_layout_future_hints_preserve_guards_after_an_earlier_fault() {
            let pages = Pages::new(9);
            let pointers: [usize; 9] = std::array::from_fn(|row| pages.at(row) + 61);
            for (failed_row, guarded_row) in [(0, 1), (0, 4), (1, 5), (4, 8)] {
                pages.protect(failed_row, NOACCESS);
                pages.protect(guarded_row, READWRITE | GUARD);
                let mut output = [0xa5; 9 * 8];
                assert_eq!(
                    gather(&pointers, &POINTER_FIELDS, 9, 8, &mut output),
                    failed_row
                );
                for row in 0..failed_row {
                    assert_eq!(&output[row * 8..row * 8 + 8], &[0x20 + row as u8; 8]);
                }
                assert!(output[failed_row * 8..].iter().all(|&byte| byte == 0xa5));
                assert_eq!(pages.protect(failed_row, READWRITE), NOACCESS);
                assert_eq!(pages.protect(guarded_row, READWRITE), READWRITE | GUARD);
                // Establish that the preserved protection is a real armed
                // guard, consumed only by a later actual 8-byte guarded read.
                pages.protect(guarded_row, READWRITE | GUARD);
                // SAFETY: only the native guard reads the owned protected page.
                let memory = unsafe { LiveMemory::new() };
                let mut actual = [0xa5; 8];
                assert!(!memory.copy(pointers[guarded_row], &mut actual));
                assert_eq!(actual, [0xa5; 8]);
                assert_eq!(pages.protect(guarded_row, READWRITE), READWRITE);
            }
        }

        #[test]
        fn pointer_layout_duplicates_and_empty_prefix_never_read_metadata_suffixes() {
            const ROWS: usize = 9;
            let metadata = Pages::new(2);
            metadata.protect(1, NOACCESS);
            let layout_metadata = Pages::new(2);
            layout_metadata.protect(1, NOACCESS);
            let field_start = layout_metadata.at(1) - size_of::<RecordField>();
            // SAFETY: this aligned complete field is inside the writable first
            // page. The immediately following field would enter NOACCESS.
            unsafe { (field_start as *mut RecordField).write(POINTER_FIELDS[0]) };
            // SAFETY: the initialized immutable field remains owned and alive.
            let fields =
                unsafe { std::slice::from_raw_parts(field_start as *const RecordField, 1) };
            let source: [u8; 160] = std::array::from_fn(|index| index as u8);
            let offsets = [61, 4, 61, 127, 4, 61, 0, 63, 61];
            for prefix in 0..=ROWS {
                let pointer_start = metadata.at(1) - prefix * size_of::<usize>();
                for (row, offset) in offsets[..prefix].iter().enumerate() {
                    // SAFETY: every initialized metadata word precedes the
                    // inaccessible page; duplicate values remain separate rows.
                    unsafe {
                        (pointer_start as *mut usize)
                            .add(row)
                            .write(source.as_ptr() as usize + offset)
                    };
                }
                // SAFETY: exactly prefix aligned initialized pointer words are
                // readable. The zero-length slice never accesses its address.
                let pointers =
                    unsafe { std::slice::from_raw_parts(pointer_start as *const usize, prefix) };
                let mut output = [0xa5; ROWS * 8 + 2];
                assert_eq!(
                    gather(pointers, fields, prefix, 8, &mut output[1..1 + ROWS * 8]),
                    prefix
                );
                let mut expected = [0xa5; ROWS * 8 + 2];
                for (row, &offset) in offsets[..prefix].iter().enumerate() {
                    expected[1 + row * 8..1 + row * 8 + 8]
                        .copy_from_slice(&source[offset..offset + 8]);
                }
                assert_eq!(output, expected, "prefix={prefix}");
            }
            // SAFETY: an empty native prefix accesses no metadata or output,
            // including when field_count would otherwise select this layout.
            // These owned inaccessible addresses deliberately prove that rule.
            assert_eq!(
                unsafe {
                    seh_guarded_copy_records(
                        metadata.at(1) as *const usize,
                        0,
                        0,
                        0,
                        layout_metadata.at(1) as *const RecordField,
                        1,
                        8,
                        metadata.at(1) as *mut u8,
                    )
                },
                0
            );
        }

        #[test]
        fn pointer_layout_nonmatching_fields_keep_generic_destinations_exact() {
            let first = POINTER_FIELDS[0];
            let second = RecordField {
                source_offset: 8,
                destination_offset: 0,
                len: 8,
            };
            let layouts = [
                (
                    [
                        RecordField {
                            source_offset: 1,
                            ..first
                        },
                        second,
                    ],
                    1,
                    8,
                ),
                (
                    [
                        RecordField {
                            destination_offset: 1,
                            ..first
                        },
                        second,
                    ],
                    1,
                    9,
                ),
                ([RecordField { len: 7, ..first }, second], 1, 8),
                ([first, second], 1, 9),
                // The second field overwrites the first within each row; it
                // must not be skipped merely because fields[0] matches.
                ([first, second], 2, 8),
            ];
            let source: [u8; 64] = std::array::from_fn(|index| index as u8);
            let pointers = [source.as_ptr() as usize, source.as_ptr() as usize + 32];
            // SAFETY: every source span belongs to the live owned source array.
            let memory = unsafe { LiveMemory::new() };
            for (layout, field_count, stride) in layouts {
                let fields = &layout[..field_count];
                let mut output = [0xa5; 32];
                assert_eq!(
                    memory.copy_records(
                        RecordSources::Pointers(&pointers),
                        &RecordLayout::new(fields, stride).expect("valid record layout"),
                        &mut output
                    ),
                    2 * field_count
                );
                let mut expected = [0xa5; 32];
                for row in 0..2 {
                    for field in fields {
                        let start = row * stride + field.destination_offset;
                        let source_start = row * 32 + field.source_offset;
                        expected[start..start + field.len]
                            .copy_from_slice(&source[source_start..source_start + field.len]);
                    }
                }
                assert_eq!(output, expected);
            }
        }

        fn packed_two_fields(offsets: [usize; 2]) -> [RecordField; 2] {
            [
                RecordField {
                    source_offset: offsets[0],
                    destination_offset: 0,
                    len: 2,
                },
                RecordField {
                    source_offset: offsets[1],
                    destination_offset: 8,
                    len: 8,
                },
            ]
        }

        #[test]
        fn packed_two_dynamic_offsets_duplicates_and_every_metadata_prefix() {
            const ROWS: usize = 65;
            let metadata = Pages::new(2);
            metadata.protect(1, NOACCESS);
            let layouts = Pages::new(2);
            layouts.protect(1, NOACCESS);
            let layout_start = layouts.at(1) - 2 * size_of::<RecordField>();
            let row_offsets = [61, 4, 61, 127, 4, 61, 0, 63, 61];
            // SAFETY: every live source below belongs to the owned source array.
            let memory = unsafe { LiveMemory::new() };
            for epoch in 0..2 {
                let source: [u8; 1024] =
                    std::array::from_fn(|index| (index * 37 + epoch * 73) as u8);
                for offsets in [[7, 74], [7, 7], [7, 8], [31, 3], [0x158, 8]] {
                    let layout = packed_two_fields(offsets);
                    for (index, field) in layout.iter().enumerate() {
                        // SAFETY: exactly two initialized aligned fields fit
                        // before the owned inaccessible metadata suffix.
                        unsafe { (layout_start as *mut RecordField).add(index).write(*field) };
                    }
                    // SAFETY: only the two initialized readable fields are borrowed.
                    let fields = unsafe {
                        std::slice::from_raw_parts(layout_start as *const RecordField, 2)
                    };
                    for prefix in 0..=ROWS * 2 {
                        let rows = prefix.div_ceil(2);
                        let pointer_start = metadata.at(1) - rows * size_of::<usize>();
                        for row in 0..rows {
                            // SAFETY: only the exact metadata prefix is initialized,
                            // including the final pointer of an odd partial row.
                            unsafe {
                                (pointer_start as *mut usize).add(row).write(
                                    source.as_ptr() as usize + row_offsets[row % row_offsets.len()],
                                );
                            }
                        }
                        // SAFETY: exactly rows initialized pointer words are readable.
                        let pointers = unsafe {
                            std::slice::from_raw_parts(pointer_start as *const usize, rows)
                        };
                        let mut output = [0xa5; ROWS * 16 + 2];
                        assert_eq!(
                            gather(pointers, fields, prefix, 16, &mut output[1..1 + ROWS * 16]),
                            prefix
                        );
                        let mut expected = [0xa5; ROWS * 16 + 2];
                        for index in 0..prefix {
                            let row = index / 2;
                            let field = fields[index % 2];
                            let source_start =
                                row_offsets[row % row_offsets.len()] + field.source_offset;
                            let start = 1 + row * 16 + field.destination_offset;
                            expected[start..start + field.len]
                                .copy_from_slice(&source[source_start..source_start + field.len]);
                        }
                        assert_eq!(
                            output, expected,
                            "epoch={epoch} offsets={offsets:?} prefix={prefix}"
                        );
                    }
                    // The same exact specialization also receives strided sources;
                    // seven-byte rows overlap and make alignment vary on each row.
                    for count in [0, 1, 2, 3, 4, 5, 63, 64, 65] {
                        let mut output = [0xa5; ROWS * 16 + 2];
                        assert_eq!(
                            memory.copy_records(
                                RecordSources::Strided {
                                    base: source.as_ptr() as usize + 1,
                                    stride: 7,
                                    count,
                                },
                                &RecordLayout::new(fields, 16).unwrap(),
                                &mut output[1..1 + ROWS * 16],
                            ),
                            count * 2
                        );
                        let mut expected = [0xa5; ROWS * 16 + 2];
                        for row in 0..count {
                            for field in fields {
                                let source_start = 1 + row * 7 + field.source_offset;
                                let start = 1 + row * 16 + field.destination_offset;
                                expected[start..start + field.len].copy_from_slice(
                                    &source[source_start..source_start + field.len],
                                );
                            }
                        }
                        assert_eq!(
                            output, expected,
                            "epoch={epoch} offsets={offsets:?} count={count}"
                        );
                    }
                }
            }
            // SAFETY: zero admitted fields must inspect no metadata or output.
            assert_eq!(
                unsafe {
                    seh_guarded_copy_records(
                        metadata.at(1) as *const usize,
                        0,
                        0,
                        0,
                        layouts.at(1) as *const RecordField,
                        2,
                        16,
                        metadata.at(1) as *mut u8,
                    )
                },
                0
            );
        }

        #[test]
        fn packed_two_every_owned_source_fault_preserves_the_exact_prefix() {
            const ROWS: usize = 9;
            let pages = Pages::new(ROWS * 4);
            let pointers: [usize; ROWS] = std::array::from_fn(|row| pages.at(row * 4));
            let widths = [2, 8];
            for last_byte in [false, true] {
                let fields = packed_two_fields(std::array::from_fn(|field| {
                    field * 2 * PAGE
                        + if last_byte {
                            PAGE - widths[field] + 1
                        } else {
                            61
                        }
                }));
                for failed in 0..ROWS * 2 {
                    let failed_page = failed / 2 * 4 + failed % 2 * 2 + usize::from(last_byte);
                    for protection in [NOACCESS, READWRITE | GUARD] {
                        for prefix in 0..=ROWS * 2 {
                            pages.protect(failed_page, protection);
                            let mut output = [0xa5; ROWS * 16 + 2];
                            let complete = prefix.min(failed);
                            assert_eq!(
                                gather(
                                    &pointers,
                                    &fields,
                                    prefix,
                                    16,
                                    &mut output[1..1 + ROWS * 16],
                                ),
                                complete,
                                "last_byte={last_byte} failed={failed} prefix={prefix}"
                            );
                            assert_eq!(
                                pages.protect(failed_page, READWRITE),
                                if protection & GUARD != 0 && prefix > failed {
                                    READWRITE
                                } else {
                                    protection
                                },
                                "unreached and odd-prefix excluded guards must remain armed"
                            );
                            let mut expected = [0xa5; ROWS * 16 + 2];
                            for index in 0..complete {
                                let row = index / 2;
                                let field = fields[index % 2];
                                for byte in 0..field.len {
                                    expected[1 + row * 16 + field.destination_offset + byte] = 0x20
                                        + (row * 4 + (field.source_offset + byte) / PAGE) as u8;
                                }
                            }
                            if prefix > failed {
                                // Only the failed field may be partial. Padding,
                                // following fields and exterior canaries stay exact.
                                let field = fields[failed % 2];
                                let start = 1 + failed / 2 * 16 + field.destination_offset;
                                output[start..start + field.len].fill(0xa5);
                            }
                            assert_eq!(output, expected);
                        }
                    }
                }
            }

            // Every exact field ends at the last readable byte. Widening
            // either the two-byte or eight-byte load must cross NOACCESS.
            let fields = packed_two_fields(std::array::from_fn(|field| {
                field * 2 * PAGE + PAGE - widths[field]
            }));
            for pair in 0..ROWS * 2 {
                pages.protect(pair * 2 + 1, NOACCESS);
            }
            let mut output = [0xa5; ROWS * 16 + 2];
            assert_eq!(
                gather(
                    &pointers,
                    &fields,
                    ROWS * 2,
                    16,
                    &mut output[1..1 + ROWS * 16],
                ),
                ROWS * 2
            );
            let mut expected = [0xa5; ROWS * 16 + 2];
            for row in 0..ROWS {
                for (index, field) in fields.iter().enumerate() {
                    let start = 1 + row * 16 + field.destination_offset;
                    expected[start..start + field.len].fill(0x20 + (row * 4 + index * 2) as u8);
                }
            }
            assert_eq!(output, expected);
            for pair in 0..ROWS * 2 {
                assert_eq!(pages.protect(pair * 2 + 1, READWRITE), NOACCESS);
            }
        }

        #[test]
        fn packed_two_future_endpoint_hints_do_not_consume_guards() {
            const ROWS: usize = 9;
            let pages = Pages::new(ROWS * 2);
            let pointers: [usize; ROWS] = std::array::from_fn(|row| pages.at(row * 2));
            let fields = packed_two_fields([63, PAGE + 61]);
            for (failed_row, future_row) in [(0, 1), (0, 4), (1, 5), (4, 8)] {
                for failed_field in 0..2 {
                    for future_field in 0..2 {
                        let failed_page = failed_row * 2 + failed_field;
                        let future_page = future_row * 2 + future_field;
                        pages.protect(failed_page, NOACCESS);
                        pages.protect(future_page, READWRITE | GUARD);
                        let mut output = [0xa5; ROWS * 16];
                        assert_eq!(
                            gather(&pointers, &fields, ROWS * 2, 16, &mut output),
                            failed_row * 2 + failed_field
                        );
                        assert_eq!(pages.protect(failed_page, READWRITE), NOACCESS);
                        assert_eq!(pages.protect(future_page, READWRITE), READWRITE | GUARD);
                        pages.protect(future_page, READWRITE | GUARD);
                        // SAFETY: only an actual guarded source copy may consume
                        // this owned guard; no protected page is Rust-borrowed.
                        let memory = unsafe { LiveMemory::new() };
                        let mut bytes = [0xa5; 8];
                        assert!(!memory.copy(
                            pointers[future_row] + fields[future_field].source_offset,
                            &mut bytes[..fields[future_field].len],
                        ));
                        assert_eq!(bytes, [0xa5; 8]);
                        assert_eq!(pages.protect(future_page, READWRITE), READWRITE);
                    }
                }
            }
        }

        #[test]
        fn packed_two_layout_mismatches_keep_generic_order_and_padding() {
            let source: [u8; 1024] = std::array::from_fn(|index| (index * 37) as u8);
            let pointers = [
                source.as_ptr() as usize + 13,
                source.as_ptr() as usize + 513,
            ];
            let base = packed_two_fields([31, 3]);
            let mut layouts = vec![(base.to_vec(), 16), (base.to_vec(), 17)];
            for index in 0..2 {
                let mut fields = base;
                fields[index].len -= 1;
                layouts.push((fields.to_vec(), 16));
                let mut fields = base;
                fields[index].destination_offset = if index == 0 { 1 } else { 7 };
                layouts.push((fields.to_vec(), 16));
            }
            let mut overlap = base;
            overlap[1].destination_offset = 1;
            layouts.push((overlap.to_vec(), 16));
            layouts.push((base[..1].to_vec(), 16));
            layouts.push((vec![], 16));
            let mut fields = base.to_vec();
            fields.push(RecordField {
                source_offset: usize::MAX,
                destination_offset: 16,
                len: 0,
            });
            layouts.push((fields, 16));
            // SAFETY: every nonempty source belongs to the owned source array.
            let memory = unsafe { LiveMemory::new() };
            for (fields, stride) in layouts {
                let mut output = [0xa5; 36];
                assert_eq!(
                    memory.copy_records(
                        RecordSources::Pointers(&pointers),
                        &RecordLayout::new(&fields, stride).expect("valid record layout"),
                        &mut output[1..35],
                    ),
                    2 * fields.len()
                );
                let mut expected = [0xa5; 36];
                for row in 0..2 {
                    for field in &fields {
                        if field.len != 0 {
                            let source_start = [13, 513][row] + field.source_offset;
                            let start = 1 + row * stride + field.destination_offset;
                            expected[start..start + field.len]
                                .copy_from_slice(&source[source_start..source_start + field.len]);
                        }
                    }
                }
                assert_eq!(output, expected);
            }
        }

        fn packed_five_fields(offsets: [usize; 5]) -> [RecordField; 5] {
            let widths = [12, 16, 8, 8, 8];
            let destinations = [0, 12, 28, 36, 44];
            std::array::from_fn(|index| RecordField {
                source_offset: offsets[index],
                destination_offset: destinations[index],
                len: widths[index],
            })
        }

        #[test]
        fn packed_five_dynamic_offsets_duplicates_and_every_metadata_prefix() {
            const ROWS: usize = 9;
            let metadata = Pages::new(2);
            metadata.protect(1, NOACCESS);
            let layouts = Pages::new(2);
            layouts.protect(1, NOACCESS);
            let layout_start = layouts.at(1) - 5 * size_of::<RecordField>();
            for epoch in 0..2 {
                let source: [u8; 1024] =
                    std::array::from_fn(|index| (index * 37 + epoch * 73) as u8);
                for offsets in [[0, 0x158, 0x170, 0x198, 0x1c0], [7, 74, 11, 259, 31]] {
                    let layout = packed_five_fields(offsets);
                    for (index, field) in layout.iter().enumerate() {
                        // SAFETY: five aligned initialized fields fit before the
                        // owned inaccessible page; a sixth field does not.
                        unsafe { (layout_start as *mut RecordField).add(index).write(*field) };
                    }
                    // SAFETY: only the five initialized readable fields are borrowed.
                    let fields = unsafe {
                        std::slice::from_raw_parts(layout_start as *const RecordField, 5)
                    };
                    let offsets = [61, 4, 61, 127, 4, 61, 0, 63, 61];
                    for prefix in 0..=ROWS * 5 {
                        let rows = prefix.div_ceil(5);
                        let pointer_start = metadata.at(1) - rows * size_of::<usize>();
                        for (row, offset) in offsets[..rows].iter().enumerate() {
                            // SAFETY: exactly the needed initialized pointer words
                            // precede the inaccessible page, including partial rows.
                            unsafe {
                                (pointer_start as *mut usize)
                                    .add(row)
                                    .write(source.as_ptr() as usize + offset);
                            }
                        }
                        // SAFETY: only initialized metadata is borrowed; the empty
                        // slice is aligned/non-null but its address is never read.
                        let pointers = unsafe {
                            std::slice::from_raw_parts(pointer_start as *const usize, rows)
                        };
                        let mut output = [0xa5; ROWS * 52 + 2];
                        assert_eq!(
                            gather(pointers, fields, prefix, 52, &mut output[1..1 + ROWS * 52]),
                            prefix
                        );
                        let mut expected = [0xa5; ROWS * 52 + 2];
                        for index in 0..prefix {
                            let row = index / 5;
                            let field = fields[index % 5];
                            let source_start = offsets[row] + field.source_offset;
                            let start = 1 + row * 52 + field.destination_offset;
                            expected[start..start + field.len]
                                .copy_from_slice(&source[source_start..source_start + field.len]);
                        }
                        assert_eq!(output, expected, "epoch={epoch} prefix={prefix}");
                    }
                }
            }
            // SAFETY: an empty native prefix must inspect no metadata or output.
            // All three raw addresses deliberately point into owned NOACCESS pages.
            assert_eq!(
                unsafe {
                    seh_guarded_copy_records(
                        metadata.at(1) as *const usize,
                        0,
                        0,
                        0,
                        layouts.at(1) as *const RecordField,
                        5,
                        52,
                        metadata.at(1) as *mut u8,
                    )
                },
                0
            );
        }

        #[test]
        fn packed_five_every_owned_source_fault_preserves_the_exact_prefix() {
            const ROWS: usize = 9;
            let pages = Pages::new(ROWS * 10);
            let pointers: [usize; ROWS] = std::array::from_fn(|row| pages.at(row * 10));
            for last_byte in [false, true] {
                let widths = [12, 16, 8, 8, 8];
                let fields = packed_five_fields(std::array::from_fn(|field| {
                    field * 2 * PAGE
                        + if last_byte {
                            PAGE - widths[field] + 1
                        } else {
                            61
                        }
                }));
                for failed in 0..ROWS * 5 {
                    let failed_page = failed / 5 * 10 + failed % 5 * 2 + usize::from(last_byte);
                    for protection in [NOACCESS, READWRITE | GUARD] {
                        for prefix in 0..=ROWS * 5 {
                            pages.protect(failed_page, protection);
                            let mut output = [0xa5; ROWS * 52 + 2];
                            let complete = prefix.min(failed);
                            assert_eq!(
                                gather(
                                    &pointers,
                                    &fields,
                                    prefix,
                                    52,
                                    &mut output[1..1 + ROWS * 52]
                                ),
                                complete,
                                "last_byte={last_byte} failed={failed} prefix={prefix}"
                            );
                            let old = pages.protect(failed_page, READWRITE);
                            assert_eq!(
                                old,
                                if protection & GUARD != 0 && prefix > failed {
                                    READWRITE
                                } else {
                                    protection
                                },
                                "hints must not consume an unreached guard"
                            );
                            let mut expected = [0xa5; ROWS * 52 + 2];
                            for index in 0..complete {
                                let row = index / 5;
                                let field = fields[index % 5];
                                for byte in 0..field.len {
                                    expected[1 + row * 52 + field.destination_offset + byte] = 0x20
                                        + (row * 10 + (field.source_offset + byte) / PAGE) as u8;
                                }
                            }
                            if prefix > failed {
                                // The failing field may be partially written. Only
                                // that exact field is unspecified by Memory; all
                                // later fields and both exterior canaries must match.
                                let field = fields[failed % 5];
                                let start = 1 + failed / 5 * 52 + field.destination_offset;
                                output[start..start + field.len].fill(0xa5);
                            }
                            assert_eq!(output, expected);
                        }
                    }
                }
            }

            // Positive exact-range boundary: every intended field is readable,
            // but its immediately following byte/page is not. Any widened
            // 12->16, 16->32 or 8->16 load across that exact end must fail.
            let widths = [12, 16, 8, 8, 8];
            let fields = packed_five_fields(std::array::from_fn(|field| {
                field * 2 * PAGE + PAGE - widths[field]
            }));
            for pair in 0..ROWS * 5 {
                pages.protect(pair * 2 + 1, NOACCESS);
            }
            let mut output = [0xa5; ROWS * 52 + 2];
            assert_eq!(
                gather(
                    &pointers,
                    &fields,
                    ROWS * 5,
                    52,
                    &mut output[1..1 + ROWS * 52]
                ),
                ROWS * 5
            );
            let mut expected = [0xa5; ROWS * 52 + 2];
            for row in 0..ROWS {
                for (index, field) in fields.iter().enumerate() {
                    let start = 1 + row * 52 + field.destination_offset;
                    expected[start..start + field.len].fill(0x20 + (row * 10 + index * 2) as u8);
                }
            }
            assert_eq!(output, expected);
            for pair in 0..ROWS * 5 {
                assert_eq!(pages.protect(pair * 2 + 1, READWRITE), NOACCESS);
            }
        }

        #[test]
        fn packed_five_future_endpoint_hints_do_not_consume_guards() {
            const ROWS: usize = 9;
            let pages = Pages::new(ROWS * 5);
            let pointers: [usize; ROWS] = std::array::from_fn(|row| pages.at(row * 5));
            let fields = packed_five_fields(std::array::from_fn(|field| field * PAGE + 61));
            for (failed_row, future_row) in [(0, 4), (1, 5), (4, 8)] {
                for failed_field in 0..5 {
                    for future_field in 0..5 {
                        let failed_page = failed_row * 5 + failed_field;
                        let future_page = future_row * 5 + future_field;
                        pages.protect(failed_page, NOACCESS);
                        pages.protect(future_page, READWRITE | GUARD);
                        let mut output = [0xa5; ROWS * 52];
                        assert_eq!(
                            gather(&pointers, &fields, ROWS * 5, 52, &mut output),
                            failed_row * 5 + failed_field
                        );
                        assert_eq!(pages.protect(failed_page, READWRITE), NOACCESS);
                        assert_eq!(pages.protect(future_page, READWRITE), READWRITE | GUARD);
                        pages.protect(future_page, READWRITE | GUARD);
                        // SAFETY: the actual native read, not the hint, must consume
                        // this real owned guard. No protected page is Rust-borrowed.
                        let memory = unsafe { LiveMemory::new() };
                        let mut bytes = [0xa5; 16];
                        assert!(!memory.copy(
                            pointers[future_row] + fields[future_field].source_offset,
                            &mut bytes[..fields[future_field].len]
                        ));
                        assert_eq!(bytes, [0xa5; 16]);
                        assert_eq!(pages.protect(future_page, READWRITE), READWRITE);
                    }
                }
            }
        }

        #[test]
        fn packed_five_layout_mismatches_keep_generic_order_and_padding() {
            let source: [u8; 1024] = std::array::from_fn(|index| (index * 37) as u8);
            let pointers = [
                source.as_ptr() as usize + 13,
                source.as_ptr() as usize + 513,
            ];
            let base = packed_five_fields([31, 3, 127, 61, 1]);
            let mut layouts = vec![(base.to_vec(), 52), (base.to_vec(), 53)];
            for index in 0..5 {
                let mut fields = base;
                fields[index].len -= 1;
                layouts.push((fields.to_vec(), 52));
                let mut fields = base;
                fields[index].destination_offset = if index == 0 {
                    1
                } else {
                    fields[index].destination_offset - 1
                };
                layouts.push((fields.to_vec(), 52));
            }
            layouts.push((base[..4].to_vec(), 52));
            let mut fields = base.to_vec();
            fields.push(RecordField {
                source_offset: usize::MAX,
                destination_offset: 52,
                len: 0,
            });
            layouts.push((fields, 52));
            // SAFETY: every nonempty source belongs to the owned source array.
            let memory = unsafe { LiveMemory::new() };
            for (fields, stride) in layouts {
                let mut output = [0xa5; 110];
                assert_eq!(
                    memory.copy_records(
                        RecordSources::Pointers(&pointers),
                        &RecordLayout::new(&fields, stride).expect("valid record layout"),
                        &mut output[1..109]
                    ),
                    2 * fields.len()
                );
                let mut expected = [0xa5; 110];
                for row in 0..2 {
                    for field in &fields {
                        if field.len != 0 {
                            let source_start = [13, 513][row] + field.source_offset;
                            let start = 1 + row * stride + field.destination_offset;
                            expected[start..start + field.len]
                                .copy_from_slice(&source[source_start..source_start + field.len]);
                        }
                    }
                }
                assert_eq!(output, expected);
            }
        }

        #[test]
        fn packed_five_canonical_preflight_and_native_minus_one_containment() {
            const NONCANONICAL: usize = 0x0000_8000_0000_0000;
            let source: [u8; 128] = std::array::from_fn(|index| (index * 37) as u8);
            let fields = packed_five_fields([0, 16, 32, 40, 48]);
            let valid = source.as_ptr() as usize;
            // SAFETY: only owned readable bytes, or deliberately invalid addresses
            // rejected by Rust/contained by the native guard, are provided below.
            let memory = unsafe { LiveMemory::new() };
            for invalid in [0, NONCANONICAL, usize::MAX] {
                let pointers = [valid, invalid, valid];
                let mut output = [0xa5; 3 * 52];
                assert_eq!(
                    record_source_prefix(
                        RecordSources::Pointers(&pointers),
                        &RecordLayout::new(&fields, usize::MAX).unwrap()
                    ),
                    5
                );
                assert_eq!(
                    memory.copy_records(
                        RecordSources::Pointers(&pointers),
                        &RecordLayout::new(&fields, 52).expect("valid record layout"),
                        &mut output
                    ),
                    5
                );
                let mut expected = [0xa5; 3 * 52];
                for field in fields {
                    expected[field.destination_offset..field.destination_offset + field.len]
                        .copy_from_slice(
                            &source[field.source_offset..field.source_offset + field.len],
                        );
                }
                assert_eq!(output, expected);
            }
            for invalid_field in 0..5 {
                let mut layout = fields;
                layout[invalid_field].source_offset = usize::MAX;
                let mut output = [0xa5; 52];
                assert_eq!(
                    record_source_prefix(
                        RecordSources::Pointers(&[valid]),
                        &RecordLayout::new(&layout, usize::MAX).unwrap()
                    ),
                    invalid_field
                );
                assert_eq!(
                    memory.copy_records(
                        RecordSources::Pointers(&[valid]),
                        &RecordLayout::new(&layout, 52).expect("valid record layout"),
                        &mut output
                    ),
                    invalid_field
                );
                let mut expected = [0xa5; 52];
                for field in &fields[..invalid_field] {
                    expected[field.destination_offset..field.destination_offset + field.len]
                        .copy_from_slice(
                            &source[field.source_offset..field.source_offset + field.len],
                        );
                }
                assert_eq!(output, expected);
            }
            let pages = Pages::new(1);
            pages.protect(0, NOACCESS);
            let mut layout = fields;
            layout[4].source_offset = NONCANONICAL.checked_sub(pages.at(0)).unwrap();
            let mut output = [0xa5; 52];
            assert_eq!(
                record_source_prefix(
                    RecordSources::Pointers(&[pages.at(0)]),
                    &RecordLayout::new(&layout, usize::MAX).unwrap()
                ),
                4
            );
            assert_eq!(
                memory.copy_records(
                    RecordSources::Pointers(&[pages.at(0)]),
                    &RecordLayout::new(&layout, 52).expect("valid record layout"),
                    &mut output
                ),
                0,
                "earlier owned unreadable field beats a later noncanonical field"
            );
            assert_eq!(output, [0xa5; 52]);
            assert_eq!(pages.protect(0, READWRITE), NOACCESS);
            let bad = [0x0072_6f74_6361_665fusize];
            // SAFETY: this direct containment test intentionally bypasses only
            // Rust's canonical gate, as the existing scalar -1 fixture does.
            // Metadata/output are valid; only C touches the invalid source.
            assert_eq!(
                unsafe {
                    seh_guarded_copy_records(
                        bad.as_ptr(),
                        0,
                        0,
                        5,
                        fields.as_ptr(),
                        5,
                        52,
                        output.as_mut_ptr(),
                    )
                },
                0
            );
            assert_eq!(output, [0xa5; 52]);
        }

        #[test]
        fn packed_five_owned_destination_faults_keep_previous_fields_and_canaries() {
            let source: [u8; 128] = std::array::from_fn(|index| (index * 37) as u8);
            let fields = packed_five_fields([0, 16, 32, 40, 48]);
            let pointers = [source.as_ptr() as usize];
            let pages = Pages::new(3);
            for failed_field in 0..5 {
                for last_byte in [false, true] {
                    let field = fields[failed_field];
                    let fault_offset =
                        field.destination_offset + if last_byte { field.len - 1 } else { 0 };
                    let destination = (pages.at(1) - fault_offset) as *mut u8;
                    for protection in [NOACCESS, READWRITE | GUARD] {
                        // SAFETY: this full destination and both canaries are
                        // within writable owned pages before protection changes.
                        unsafe { std::ptr::write_bytes(destination.sub(1), 0xa5, 54) };
                        pages.protect(1, protection);
                        // SAFETY: metadata/source are valid; destination faults
                        // are deliberately tested through C raw pointers only.
                        // No Rust reference covers inaccessible destination bytes.
                        let completed = unsafe {
                            seh_guarded_copy_records(
                                pointers.as_ptr(),
                                0,
                                0,
                                5,
                                fields.as_ptr(),
                                5,
                                52,
                                destination,
                            )
                        };
                        let old = pages.protect(1, READWRITE);
                        assert_eq!(completed, failed_field);
                        assert_eq!(
                            old,
                            if protection & GUARD != 0 {
                                READWRITE
                            } else {
                                protection
                            }
                        );
                        // SAFETY: protection is restored before borrowing bytes;
                        // that borrow ends before the next reset/protection change.
                        let mut actual: [u8; 54] = unsafe {
                            std::slice::from_raw_parts(destination.sub(1), 54)
                                .try_into()
                                .unwrap()
                        };
                        let mut expected = [0xa5; 54];
                        for earlier in &fields[..failed_field] {
                            let start = 1 + earlier.destination_offset;
                            expected[start..start + earlier.len].copy_from_slice(
                                &source[earlier.source_offset..earlier.source_offset + earlier.len],
                            );
                        }
                        let start = 1 + field.destination_offset;
                        actual[start..start + field.len].fill(0xa5);
                        assert_eq!(actual, expected, "field={failed_field} last={last_byte}");
                    }
                }
            }
        }

        #[test]
        fn partial_final_rows_and_duplicate_fields_never_read_a_pointer_suffix() {
            const ROWS: usize = 9;
            const STRIDE: usize = 40;
            let metadata = Pages::new(2);
            metadata.protect(1, NOACCESS);
            let source: [u8; 160] = std::array::from_fn(|index| index as u8);
            let fields = [
                RecordField {
                    source_offset: usize::MAX,
                    destination_offset: 0,
                    len: 0,
                },
                RecordField {
                    source_offset: 61,
                    destination_offset: 1,
                    len: 12,
                },
                RecordField {
                    source_offset: 4,
                    destination_offset: 16,
                    len: 8,
                },
                RecordField {
                    source_offset: 61,
                    destination_offset: 25,
                    len: 12,
                },
                RecordField {
                    source_offset: usize::MAX,
                    destination_offset: STRIDE,
                    len: 0,
                },
            ];
            for prefix in 0..=ROWS * fields.len() {
                let rows = prefix.div_ceil(fields.len());
                let pointer_start = metadata.at(1) - rows * size_of::<usize>();
                for row in 0..rows {
                    // SAFETY: the pointer metadata lies in the owned writable
                    // first page; the immediately following page is NOACCESS.
                    unsafe {
                        (pointer_start as *mut usize)
                            .add(row)
                            .write(source.as_ptr() as usize)
                    };
                }
                // SAFETY: these initialized aligned pointer words are readable
                // until this call ends. For zero rows the aligned nonnull
                // address is never accessed. No slice includes the next page.
                let pointers =
                    unsafe { std::slice::from_raw_parts(pointer_start as *const usize, rows) };
                let mut output = [0xa5; ROWS * STRIDE];
                assert_eq!(
                    gather(pointers, &fields, prefix, STRIDE, &mut output),
                    prefix
                );
                let mut expected = [0xa5; ROWS * STRIDE];
                for index in 0..prefix {
                    let field = fields[index % fields.len()];
                    if field.len != 0 {
                        let start = index / fields.len() * STRIDE + field.destination_offset;
                        expected[start..start + field.len].copy_from_slice(
                            &source[field.source_offset..field.source_offset + field.len],
                        );
                    }
                }
                assert_eq!(output, expected, "prefix={prefix}");
            }
        }
    }
}
