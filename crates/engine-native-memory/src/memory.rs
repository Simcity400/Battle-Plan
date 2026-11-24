use std::mem::{MaybeUninit, size_of};

pub use crate::read_counts::{CountedMemory, ReadCounts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct RecordField {
    pub source_offset: usize,
    pub destination_offset: usize,
    pub len: usize,
}

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

pub const fn packed_stride<const N: usize>(spans: [(usize, usize); N]) -> usize {
    let mut index = 0;
    let mut at = 0;
    while index < N {
        at += spans[index].1;
        index += 1;
    }
    at
}

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

pub fn gather_records_once(
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

#[derive(Clone, Copy)]
pub struct Gathered<'a> {
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

#[inline]
pub fn is_canonical_range(addr: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let Some(end) = addr.checked_add(len.wrapping_sub(1)) else {
        return false;
    };
    let high = addr >> 47;
    matches!(high, 0 | 0x1FFFF) && end >> 47 == high
}

#[inline(always)]
fn record_source_prefix(sources: RecordSources<'_>, layout: &RecordLayout<'_>) -> usize {
    if layout.nonempty == 0 || sources.is_empty() {
        return sources.len() * layout.fields.len();
    }
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

pub trait Memory {
    #[inline]
    fn take_deferred_reads(&self) -> ReadCounts {
        ReadCounts::take_current()
    }

    #[inline]
    fn add_deferred_reads(&self, reads: ReadCounts) {
        reads.add_current();
    }

    fn copy(&self, address: usize, destination: &mut [u8]) -> bool;

    #[inline]
    fn copy_uninit<'a>(
        &self,
        address: usize,
        destination: &'a mut [MaybeUninit<u8>],
    ) -> Option<&'a mut [u8]> {
        let destination = initialized(destination);
        self.copy(address, destination).then_some(destination)
    }

    #[inline]
    fn prefetch(&self, address: usize, len: usize) {
        crate::prefetch::span(address, len);
    }

    #[inline]
    fn read_scalar<T: Raw>(&self, address: usize) -> Option<T> {
        copy_scalar(self, address)
    }

    fn execute_read_plan(
        &self,
        plan: &crate::read_plan::PreparedPlan,
        destination: &mut [u8],
    ) -> Option<crate::read_plan::Outcome> {
        plan.execute(self, destination)
    }

    fn immutable_view(&self, _address: usize, _len: usize) -> Option<SliceMemory<'_>> {
        None
    }

    fn authenticated_countries(
        &self,
        _base: usize,
        _game_state: usize,
    ) -> Option<crate::country_identity::AuthenticatedRoster<'_>> {
        None
    }

    fn authenticated_reserved_country(
        &self,
        _base: usize,
        _game_state: usize,
    ) -> Option<Result<crate::country_identity::Country, &'static str>> {
        None
    }

    fn authenticated_reserved_country_observation(
        &self,
        _base: usize,
        _game_state: usize,
    ) -> Option<crate::country_identity::ReservedCountryObservation> {
        None
    }

    fn modifier_accumulator_value(
        &self,
        _address: usize,
        _key: u32,
    ) -> Option<Result<crate::modifier_cache::Value, &'static str>> {
        None
    }

    fn modifier_definitions(&self) -> Option<&crate::modifier_cache::PassDefinitions> {
        None
    }

    fn core_supply_access(
        &self,
        _base: usize,
        _game_state: usize,
        _owner_raw: u32,
        _area: usize,
    ) -> Option<Result<bool, &'static str>> {
        None
    }

    fn movement_primary_root(
        &self,
        _base: usize,
        _member: usize,
    ) -> Option<Result<usize, &'static str>> {
        None
    }

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
        std::slice::from_raw_parts_mut((&raw mut value).cast::<u8>(), size_of::<T>())
    };
    memory.copy(address, bytes).then_some(value)
}

#[inline]
pub fn scalar_bytes<T: Raw>(value: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(std::ptr::from_ref(value).cast::<u8>(), size_of::<T>()) }
}

#[inline]
pub fn scalar_from_bytes<T: Raw>(source: &[u8]) -> Option<T> {
    if source.len() != size_of::<T>() {
        return None;
    }
    let mut value = T::ZERO;
    // SAFETY: Raw has no padding or invalid bit patterns; the exact destination
    // is initialized, writable and cannot overlap the borrowed source.
    let destination = unsafe {
        std::slice::from_raw_parts_mut((&raw mut value).cast::<u8>(), size_of::<T>())
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

#[inline(always)]
pub fn read_slice(memory: &impl Memory, address: usize, destination: &mut [u8]) -> bool {
    if destination.is_empty() {
        return true;
    }
    address != 0
        && address.checked_add(destination.len()).is_some()
        && memory.copy(address, destination)
}

pub struct FieldBatch<const N: usize> {
    addresses: [usize; N],
    lens: [usize; N],
    bytes: [[u8; FIELD_BATCH_BYTES]; N],
    completed: usize,
}

const FIELD_BATCH_BYTES: usize = 24;

impl<const N: usize> FieldBatch<N> {
    #[inline]
    pub fn gather(memory: &impl Memory, fields: [(usize, usize); N]) -> Self {
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
    pub fn get<T: Raw>(&self, memory: &impl Memory, index: usize) -> Option<T> {
        if size_of::<T>() != self.lens[index] {
            return None;
        }
        match index.cmp(&self.completed) {
            std::cmp::Ordering::Less => scalar_from_bytes(&self.bytes[index][..size_of::<T>()]),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => read(memory, self.addresses[index]),
        }
    }
}

#[inline]
pub fn read_slice_uninit<'a>(
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

#[inline]
fn initialized(bytes: &mut [MaybeUninit<u8>]) -> &mut [u8] {
    for byte in bytes.iter_mut() {
        byte.write(0);
    }
    // SAFETY: every byte was written just above, and `u8` has the layout of
    // `MaybeUninit<u8>`; the borrow is the caller's unique borrow.
    unsafe { &mut *(std::ptr::from_mut(bytes) as *mut [u8]) }
}

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
/// `destination` must be valid for `len` byte writes, `address..+len`
/// must have passed the null, overflow and canonical gates, and the two
/// ranges must not overlap. As for `LiveMemory::new`, the source is foreign
/// memory read under the caller's consistency protocol, or immutable synthetic
/// storage, for the whole copy.
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
#[inline(always)]
unsafe fn guarded_copy(destination: *mut u8, address: usize, len: usize) -> bool {
    unsafe extern "C" {
        fn seh_guarded_copy(dst: *mut u8, src: *const u8, len: usize) -> i32;
        fn seh_guarded_copy4(dst: *mut u8, src: *const u8) -> i32;
        fn seh_guarded_copy8(dst: *mut u8, src: *const u8) -> i32;
    }
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
                &raw mut completed,
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
        if !is_canonical_range(address, destination.len()) {
            return false;
        }
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
        let (pointers, source_base, source_stride) = match sources {
            RecordSources::Pointers(pointers) => (pointers.as_ptr(), 0, 0),
            RecordSources::Strided { base, stride, .. } => (std::ptr::null(), base, stride),
        };
        // SAFETY: the two immutable metadata slices and initialized writable
        // destination outlive the call. RecordField has the C layout, and the
        // checked row count/stride and field bounds cover every destination.
        // Every nonempty source within validated_field_count has passed the
        // Rust null/overflow/canonical checks. C only evaluates that prefix,
        // including a possible partial final row; invalid and later sources
        // never reach a guarded instruction. SEH still decides readability in
        // order, so an earlier unreadable field wins over a later invalid one.
        // C preserves its completed-field witness and exact fault-range filter.
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
