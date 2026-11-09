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
    pub fn count(&self, sources: RecordSources<'_>, destination_len: usize) -> Option<usize> {
        let count = sources.len().checked_mul(self.fields.len())?;
        if sources.len().checked_mul(self.stride)? > destination_len {
            return None;
        }
        Some(count)
    }
}

/// Packs `(source_offset, len)` spans back to back into record fields.
pub const fn packed_fields<const N: usize>(spans: [(usize, usize); N]) -> [RecordField; N] {
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
pub const fn packed_stride<const N: usize>(spans: [(usize, usize); N]) -> usize {
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
pub const fn packed_map<const N: usize, const M: usize>(spans: [(usize, usize); N]) -> [u16; M] {
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
pub fn gather_records(
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

/// One guarded gather, retaining only complete records before its first
/// incomplete one. The caller decodes in serial order and reads the incomplete
/// record and the rest of this batch by its original per-object path, once.
/// No partial bytes or speculative suffix can supply a published value.
pub(crate) fn gather_records_once(
    memory: &impl Memory,
    pointers: &[usize],
    layout: &RecordLayout<'_>,
    bytes: &mut [u8],
    complete: &mut [bool],
    batch: crate::batch_fallback::Batch,
) {
    complete[..pointers.len()].fill(false);
    if pointers.is_empty() {
        return;
    }
    let copied = memory.copy_records(RecordSources::Pointers(pointers), layout, bytes);
    let done = (copied / layout.field_count().max(1)).min(pointers.len());
    complete[..done].fill(true);
    if done != pointers.len() {
        crate::batch_fallback::record(batch);
    }
}

/// One object's fields from [`gather_records`], or nothing when its record
/// did not complete. Readers take a value from here first and otherwise make
/// their original per-field read, so rows and failures are those of the
/// per-field walk: every gathered byte is the byte that read would copy.
#[derive(Clone, Copy)]
pub struct Gathered<'a> {
    /// The layout's [`packed_map`].
    map: &'a [u16],
    bytes: Option<&'a [u8]>,
}

impl<'a> Gathered<'a> {
    pub const NONE: Gathered<'static> = Gathered {
        map: &[],
        bytes: None,
    };

    #[inline(always)]
    pub fn row(
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
    pub fn get<T: Raw>(self, offset: usize) -> Option<T> {
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

    /// Actual country slot zero's first authentication result in this pass.
    /// Success and failure belong to the exact module/state and hourly lease,
    /// independently of the positive census. Owned replay adapters copy this
    /// scalar; observation-recording wrappers must not silently retry a failure.
    fn authenticated_reserved_country(
        &self,
        _base: usize,
        _game_state: usize,
    ) -> Option<Result<crate::ccountry::Country, &'static str>> {
        None
    }

    /// The same pass-local reserved result with its original slot selection.
    /// This optional capability performs no source read. Its module/state and
    /// quiescent lease are exactly those of authenticated_reserved_country;
    /// failed selection/authentication must never be retried by a forwarder.
    fn authenticated_reserved_country_observation(
        &self,
        _base: usize,
        _game_state: usize,
    ) -> Option<crate::ccountry::ReservedCountryObservation> {
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

    /// Optional modifier metadata shared within one acquisition pass: the
    /// eligibility verdict and flags word of each key already read in full.
    /// Absence keeps the ordinary reads. Observation-recording wrappers must
    /// not forward this capability.
    fn modifier_definitions(&self) -> Option<&crate::modifier_definition::PassDefinitions> {
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
    /// Observation-recording wrappers must not forward this capability.
    fn movement_primary_root(
        &self,
        _base: usize,
        _member: usize,
    ) -> Option<Result<usize, &'static str>> {
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
    #[cfg(feature = "acquisition_profile")]
    if std::mem::size_of::<T>() == std::mem::size_of::<usize>() {
        crate::acquisition_profile::pointer_width_read();
    }
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

/// Independent scalar fields gathered in one guarded call ahead of the reader
/// that consumes them. The reader consults each field where it would otherwise
/// read it: a gathered field is served, the field the gather failed on fails
/// again without another read, and a field after that failure is read singly
/// on first use. A field the reader never consults cannot affect its result,
/// so a gather may run ahead of the branches that select its fields; callers
/// only include module globals and fields of objects the reader already
/// dereferences on that path.
pub(crate) struct FieldBatch<const N: usize> {
    addresses: [usize; N],
    lens: [usize; N],
    bytes: [[u8; FIELD_BATCH_BYTES]; N],
    completed: usize,
}

/// The widest field a batch holds: a 16-byte array header or a 17-byte hash header.
const FIELD_BATCH_BYTES: usize = 24;

impl<const N: usize> FieldBatch<N> {
    /// `fields` are (address, byte length <= 24). A null or overflowing
    /// address fails its field, exactly as a single read of it would.
    #[inline]
    pub(crate) fn gather(memory: &impl Memory, fields: [(usize, usize); N]) -> Self {
        let mut batch = Self {
            addresses: fields.map(|(address, _)| address),
            lens: fields.map(|(_, len)| {
                debug_assert!(len <= FIELD_BATCH_BYTES, "batched field too wide");
                len
            }),
            bytes: [[0; FIELD_BATCH_BYTES]; N],
            completed: 0,
        };
        let lens = batch.lens;
        let mut index = 0;
        let mut destinations = batch.bytes.each_mut().map(|bytes| {
            let len = lens[index];
            index += 1;
            &mut bytes[..len]
        });
        batch.completed = memory.copy_scattered_prefetched(batch.addresses, &mut destinations);
        batch
    }

    #[inline]
    pub(crate) fn get<T: Raw>(&self, memory: &impl Memory, index: usize) -> Option<T> {
        debug_assert_eq!(size_of::<T>(), self.lens[index]);
        match index.cmp(&self.completed) {
            std::cmp::Ordering::Less => scalar_from_bytes(&self.bytes[index][..size_of::<T>()]),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => read(memory, self.addresses[index]),
        }
    }
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

/// Test support for capture benches: counts the guarded calls a live reader
/// would make (each top-level `copy`, `copy_scattered` and `copy_records` is
/// one guard) and the bytes they read, forwarding to the wrapped memory.
#[cfg(any(test, feature = "test-support"))]
pub struct CountingMemory<'a> {
    pub inner: SliceMemory<'a>,
    pub calls: std::cell::Cell<usize>,
    pub bytes: std::cell::Cell<usize>,
}

#[cfg(any(test, feature = "test-support"))]
impl<'a> CountingMemory<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            inner: SliceMemory { base: 0, bytes },
            calls: std::cell::Cell::new(0),
            bytes: std::cell::Cell::new(0),
        }
    }

    fn count(&self, bytes: usize) {
        self.calls.set(self.calls.get() + 1);
        self.bytes.set(self.bytes.get() + bytes);
    }

    /// Runs `walk` `runs` times and prints the median wall time with the
    /// guarded calls and bytes of one run.
    pub fn bench(&self, label: &str, runs: usize, mut walk: impl FnMut(&Self) -> String) {
        let mut times = Vec::with_capacity(runs);
        let mut summary = String::new();
        for _ in 0..runs {
            self.calls.set(0);
            self.bytes.set(0);
            let started = std::time::Instant::now();
            summary = walk(self);
            times.push(started.elapsed().as_secs_f64() * 1e3);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "BENCH {label}: median {:.3} ms, {} guarded calls, {} bytes, {summary}",
            times[times.len() / 2],
            self.calls.get(),
            self.bytes.get()
        );
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Memory for CountingMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.count(destination.len());
        self.inner.copy(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.count(destinations.iter().map(|d| d.len()).sum());
        self.inner.copy_scattered(addresses, destinations)
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.count(destination.len());
        self.inner.copy_records(sources, layout, destination)
    }
}

#[cfg(test)]
mod tests;
