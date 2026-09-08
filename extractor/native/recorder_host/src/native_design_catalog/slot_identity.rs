//! Passive copied-slot identity observer for a729; no wire or scheduler.
//!
//! Ownership: caller supplies the SAME A27 acquisition reader, authenticated
//! variant/definition reach and typed effective-archetype identity. An Unsealed
//! result must not escape until that reader's complete closing pass, hour and
//! campaign/stream custody checks succeed. Run in both existing A27 captures.
//! No country/ancestry inference and no global lexer census.
//!
//! Native law: 140bc4563/457e -> definition+4d8 -> 140c86220;
//! 140c86220 uses archetype+70/+7c, stride50, row+8;
//! 1424a7300 indexes lexer strings by ID*20; 1424a8d20 rebuilds by stored ID.
//! Pins and the retained140c82b50 typed-owner receipt are in the lane handoff.
//! The old A27 ReadSet::bytes counter is NOT this API's total-work budget.

use snapshot::design_catalog::DesignTextObservation;
pub use snapshot::design_catalog::DesignSlotIdentity as SlotIdentity;
use std::mem::size_of;

pub const SENTINEL_TOKEN: u32 = 0x165;
// Operational refusal limits, not engine limits or content defaults.
pub const MAX_SLOT_ROWS: usize = snapshot::design_catalog::MAX_DESIGN_SLOT_IDENTITIES;
pub const MAX_TOKEN_BYTES: usize = snapshot::design_catalog::MAX_DESIGN_SLOT_TOKEN_BYTES;
pub const MAX_STRING_CAPACITY: u64 = 1024 * 1024;
const VARIANT_VTABLE_RVA: u64 = 0x293afb8;
const EQUIPMENT_VTABLE_RVA: u64 = 0x29218a0;
const LEXER_INITIALIZED_RVA: u64 = 0x35c8b50;
const LEXER_MAX_ID_RVA: u64 = 0x35c8b54;
const LEXER_TABLE_RVA: u64 = 0x35c8b80;
const LEXER_CAPACITY_RVA: u64 = 0x35c8b8c;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalKind {
    Address,
    Unreadable,
    Torn,
    InvalidOwner,
    MissingTypedArchetype,
    InvalidVector,
    RowBound,
    LexerUnavailable,
    InvalidTokenId,
    InvalidString,
    TextBound,
    InvalidUtf8,
    InvalidTokenText,
    DuplicateIdentity,
    ReadBudget,
    AllocationBudget,
    TraceBound,
    AllocationFailed,
    Closed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub kind: RefusalKind,
    pub field: &'static str,
}
type Result<T> = std::result::Result<T, Refusal>;
fn refusal(kind: RefusalKind, field: &'static str) -> Refusal {
    Refusal { kind, field }
}
fn add(address: u64, offset: u64) -> Result<u64> {
    address.checked_add(offset).ok_or(refusal(RefusalKind::Address, "address addition"))
}
fn heap(address: u64) -> bool {
    (0x10000000000..0x7ff000000000).contains(&address) && address % 8 == 0
}
fn heap_span(address: u64, length: usize) -> bool {
    heap(address) && address.checked_add(length as u64).is_some_and(|end| end <= 0x7ff000000000)
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

/// Implement with the parent acquisition's shared accounting/closing readset.
/// Every call, including repeats and failures, must reserve its opening AND
/// closing read before opening. Retain exact address/width/opened bytes before
/// following pointers. No uncharged Vec-returning Memory adapter is provided.
/// reserve_allocation charges requested Rust payload capacity before allocation;
/// allocator overhead and caller-owned storage belong to the parent's meter.
pub trait AcquisitionRead {
    fn read_exact(&mut self, address: u64, out: &mut [u8]) -> Result<()>;
    fn reserve_allocation(&mut self, bytes: usize) -> Result<()>;
    /// Qualify remaining worst-case work before allocating the census. The
    /// exclusive acquisition borrow prevents another helper spending it mid-call.
    fn qualify(&mut self, read_bytes: usize, calls: usize, allocation: usize, spans: usize) -> Result<()>;
}
fn word<R: AcquisitionRead>(read: &mut R, address: u64) -> Result<u32> {
    let mut b = [0; 4];
    read.read_exact(address, &mut b)?;
    Ok(u32_at(&b, 0))
}
fn pointer<R: AcquisitionRead>(read: &mut R, address: u64) -> Result<u64> {
    let mut b = [0; 8];
    read.read_exact(address, &mut b)?;
    Ok(u64_at(&b, 0))
}

/// Existing A27 registry/type70 identity and membership must already be opened
/// in this acquisition. These are process-local inputs, never portable IDs.
#[derive(Clone, Copy, Debug)]
pub struct VariantDefinition {
    pub variant: u64,
    pub definition: u64,
    pub equipment_token_raw: u32,
}

/// Supplied by CAP's exact-build typed owner reader, in this acquisition.
/// A name or a guessed ancestor cannot construct this proof. The observer
/// validates the actual +4d8 edge, vptr and key against it again.
/// Until the type/writer proof is admitted, pass None and retain that refusal.
#[derive(Clone, Copy, Debug)]
pub struct TypedArchetype {
    pub address: u64,
    pub vtable_rva: u64,
    pub token_raw: u32,
}

/// Deliberately has no Serialize implementation. The host may stage its fields
/// in an unpublished row; publication requires the shared readset and current
/// hour/campaign/custody closing checks to succeed.
#[derive(Debug, PartialEq, Eq)]
pub struct UnsealedSlotCensus {
    pub equipment_token_raw: u32,
    pub archetype_token_raw: u32,
    pub archetype_token: DesignTextObservation,
    pub slots: Vec<SlotIdentity>,
}

#[derive(Clone, Copy)]
struct LexerNames {
    table: u64,
    max_id: i32,
}
fn lexer<R: AcquisitionRead>(read: &mut R, base: u64) -> Result<LexerNames> {
    let mut initialized = [0; 1];
    read.read_exact(add(base, LEXER_INITIALIZED_RVA)?, &mut initialized)?;
    let max_id = word(read, add(base, LEXER_MAX_ID_RVA)?)? as i32;
    let table = pointer(read, add(base, LEXER_TABLE_RVA)?)?;
    let capacity = word(read, add(base, LEXER_CAPACITY_RVA)?)? as i32;
    // Same passive admission as read_pdx_lexer_token_names. All four fields
    // remain in the shared closing set, including capacity.
    if initialized[0] != 1 || max_id <= 0 || capacity <= max_id || !heap(table) {
        return Err(refusal(RefusalKind::LexerUnavailable, "lexer root"));
    }
    Ok(LexerNames { table, max_id })
}
fn script_key(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(|b|
        b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'-' | b'.' | b':'))
}
fn text<R: AcquisitionRead>(read: &mut R, address: u64) -> Result<DesignTextObservation> {
    if !heap_span(address, 32) {
        return Err(refusal(RefusalKind::InvalidString, "MSVC header address"));
    }
    let mut header = [0u8; 32];
    read.read_exact(address, &mut header)?;
    let length = u64_at(&header, 16);
    let capacity = u64_at(&header, 24);
    if length > MAX_TOKEN_BYTES as u64 {
        return Err(refusal(RefusalKind::TextBound, "token length"));
    }
    if capacity < length || capacity > MAX_STRING_CAPACITY ||
        (capacity < 16 && length > 15) {
        return Err(refusal(RefusalKind::InvalidString, "MSVC header"));
    }
    let length = length as usize;
    let mut bytes = [0; MAX_TOKEN_BYTES];
    if capacity < 16 {
        bytes[..length].copy_from_slice(&header[..length]);
    } else {
        let data = u64_at(&header, 0);
        if !heap_span(data, length) {
            return Err(refusal(RefusalKind::InvalidString, "MSVC data"));
        }
        // No terminator scan and no allocation-sized or whole-table read.
        read.read_exact(data, &mut bytes[..length])?;
    }
    let bytes = &bytes[..length];
    let valid = std::str::from_utf8(bytes)
        .map_err(|_| refusal(RefusalKind::InvalidUtf8, "token UTF-8"))?;
    if !script_key(bytes) {
        return Err(refusal(RefusalKind::InvalidTokenText, "script token"));
    }
    // Charge both output buffers before either allocation; exact UTF8+hex,
    // no lossy replacement or truncation. Temporary storage is fixed on stack.
    read.reserve_allocation(length.checked_mul(3)
        .ok_or(refusal(RefusalKind::AllocationBudget, "text capacity"))?)?;
    let mut value = String::new();
    value.try_reserve_exact(length)
        .map_err(|_| refusal(RefusalKind::AllocationFailed, "token value"))?;
    let mut utf8_hex = String::new();
    utf8_hex.try_reserve_exact(length * 2)
        .map_err(|_| refusal(RefusalKind::AllocationFailed, "token hex"))?;
    value.push_str(valid);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &b in bytes {
        utf8_hex.push(HEX[(b >> 4) as usize] as char);
        utf8_hex.push(HEX[(b & 15) as usize] as char);
    }
    Ok(DesignTextObservation::Measured { value, utf8_hex })
}
fn measured_key(text: &DesignTextObservation) -> &str {
    match text {
        DesignTextObservation::Measured { value, .. } => value,
        DesignTextObservation::Unavailable { .. } => unreachable!("text() only returns measured"),
    }
}

/// Complete copied-slot census, INCLUDING unassigned rows and raw 0x165.
/// No assignment input is used to choose rows. No category, mask or authored
/// type_override is read. No global lexer dictionary or ancestor walk.
pub fn observe<R: AcquisitionRead>(
    read: &mut R,
    base: u64,
    owner: VariantDefinition,
    typed_archetype: Option<TypedArchetype>,
) -> Result<UnsealedSlotCensus> {
    let typed = typed_archetype.ok_or(
        refusal(RefusalKind::MissingTypedArchetype, "effective archetype identity"))?;
    if !heap(owner.variant) || !heap(owner.definition) || !heap(typed.address)
        || typed.token_raw == 0 || typed.token_raw > i32::MAX as u32 {
        return Err(refusal(RefusalKind::InvalidOwner, "owner pointer"));
    }
    if pointer(read, owner.variant)? != add(base, VARIANT_VTABLE_RVA)? ||
        word(read, add(owner.variant, 0x18)?)? != owner.equipment_token_raw ||
        pointer(read, add(owner.variant, 0x3f0)?)? != owner.definition ||
        pointer(read, owner.definition)? != add(base, EQUIPMENT_VTABLE_RVA)? ||
        word(read, add(owner.definition, 8)?)? != owner.equipment_token_raw {
        return Err(refusal(RefusalKind::InvalidOwner, "A27 variant definition"));
    }
    let archetype = pointer(read, add(owner.definition, 0x4d8)?)?;
    if archetype != typed.address ||
        pointer(read, archetype)? != add(base, typed.vtable_rva)? ||
        word(read, add(archetype, 8)?)? != typed.token_raw {
        return Err(refusal(RefusalKind::InvalidOwner, "effective archetype"));
    }
    let archetype_token = text(read, add(archetype, 0x18)?)?;
    let data = pointer(read, add(archetype, 0x70)?)?;
    let capacity = word(read, add(archetype, 0x78)?)? as i32;
    let count = word(read, add(archetype, 0x7c)?)? as i32;
    if count < 0 || capacity < 0 || count > capacity {
        return Err(refusal(RefusalKind::InvalidVector, "copied slots count/capacity"));
    }
    if count as usize > MAX_SLOT_ROWS {
        return Err(refusal(RefusalKind::RowBound, "copied slots count"));
    }
    if count > 0 && !heap_span(data, count as usize * 0x50) {
        return Err(refusal(RefusalKind::InvalidVector, "copied slots data"));
    }
    let count = count as usize;
    let row_allocation = count.checked_mul(size_of::<SlotIdentity>() + 3 * MAX_TOKEN_BYTES)
        .ok_or(refusal(RefusalKind::AllocationBudget, "slot census capacity"))?;
    // Remaining lexer roots (17 bytes/4 calls), row ID + string header +
    // maximum external text (196 bytes/3 calls each), opening AND closing.
    // No read/row/storage allocation begins unless the full remainder fits.
    let (remaining_bytes, remaining_calls) = if count == 0 { (0, 0) }
        else { (2 * (17 + count * (36 + MAX_TOKEN_BYTES)), 2 * (4 + 3 * count)) };
    read.qualify(remaining_bytes, remaining_calls, row_allocation, remaining_calls / 2)?;
    let mut slots = Vec::new();
    // Output metadata before Vec reservation. Each reached row/text read also
    // independently reserves its closing work through AcquisitionRead.
    read.reserve_allocation(count.checked_mul(size_of::<SlotIdentity>())
        .ok_or(refusal(RefusalKind::AllocationBudget, "slot rows"))?)?;
    slots.try_reserve_exact(count)
        .map_err(|_| refusal(RefusalKind::AllocationFailed, "slot rows"))?;
    // A witnessed empty vector needs no lexer. Null data alone is not empty.
    if count != 0 {
        let names = lexer(read, base)?;
        for ordinal in 0..count {
            let row_offset = (ordinal as u64).checked_mul(0x50)
                .ok_or(refusal(RefusalKind::Address, "slot stride"))?;
            let raw = word(read, add(add(data, row_offset)?, 8)?)?;
            if raw == 0 || raw > i32::MAX as u32 || raw as i32 > names.max_id {
                return Err(refusal(RefusalKind::InvalidTokenId, "copied slot token"));
            }
            let entry = add(names.table, u64::from(raw) * 0x20)?;
            let token = text(read, entry)?;
            let key = measured_key(&token);
            // Ordered full census; duplicates cannot become an ambiguous
            // authored binding. Comparison follows existing ASCII token law.
            if slots.iter().any(|prior: &SlotIdentity|
                prior.token_handle_raw == raw ||
                measured_key(&prior.token).eq_ignore_ascii_case(key)) {
                return Err(refusal(RefusalKind::DuplicateIdentity, "copied slot identity"));
            }
            slots.push(SlotIdentity {
                source_ordinal: ordinal as u32,
                token_handle_raw: raw,
                token,
            });
        }
    }
    Ok(UnsealedSlotCensus {
        equipment_token_raw: owner.equipment_token_raw,
        archetype_token_raw: typed.token_raw,
        archetype_token,
        slots,
    })
}

/// Shared monotone meter. The parent lends ONE meter across owners, failed
/// attempts, helpers and both A27 captures; this observer never resets it.
/// Read work reserves two calls and twice the requested width before opening.
/// Closing consumes that reservation, not a new debit; failure never refunds.
#[derive(Debug)]
pub struct Budget {
    read_limit: usize,
    call_limit: usize,
    allocation_limit: usize,
    pub read_bytes_reserved: usize,
    pub read_calls_reserved: usize,
    pub allocation_bytes_reserved: usize,
}
impl Budget {
    pub fn new(read_bytes: usize, calls: usize, allocation_bytes: usize) -> Self {
        Self { read_limit: read_bytes, call_limit: calls, allocation_limit: allocation_bytes,
            read_bytes_reserved: 0, read_calls_reserved: 0, allocation_bytes_reserved: 0 }
    }
    pub(super) fn qualify(&self, bytes: usize, calls: usize, allocation: usize) -> Result<()> {
        if self.read_bytes_reserved.checked_add(bytes).map_or(true, |n| n > self.read_limit) ||
            self.read_calls_reserved.checked_add(calls).map_or(true, |n| n > self.call_limit) {
            return Err(refusal(RefusalKind::ReadBudget, "complete census work"));
        }
        if self.allocation_bytes_reserved.checked_add(allocation)
            .map_or(true, |n| n > self.allocation_limit) {
            return Err(refusal(RefusalKind::AllocationBudget, "complete census capacity"));
        }
        Ok(())
    }
    pub(super) fn reserve_work(&mut self, bytes: usize, calls: usize) -> Result<()> {
        self.qualify(bytes, calls, 0)?;
        self.read_bytes_reserved += bytes;
        self.read_calls_reserved += calls;
        Ok(())
    }
    #[cfg(test)]
    fn opening(&mut self, width: usize) -> Result<()> {
        let bytes = width.checked_mul(2).and_then(|n| self.read_bytes_reserved.checked_add(n))
            .ok_or(refusal(RefusalKind::ReadBudget, "opening and closing bytes"))?;
        let calls = self.read_calls_reserved.checked_add(2)
            .ok_or(refusal(RefusalKind::ReadBudget, "opening and closing calls"))?;
        if bytes > self.read_limit || calls > self.call_limit {
            return Err(refusal(RefusalKind::ReadBudget, "opening and closing work"));
        }
        self.read_bytes_reserved = bytes;
        self.read_calls_reserved = calls;
        Ok(())
    }
    pub fn allocation(&mut self, bytes: usize) -> Result<()> {
        let next = self.allocation_bytes_reserved.checked_add(bytes)
            .ok_or(refusal(RefusalKind::AllocationBudget, "allocation capacity"))?;
        if next > self.allocation_limit {
            return Err(refusal(RefusalKind::AllocationBudget, "allocation capacity"));
        }
        self.allocation_bytes_reserved = next;
        Ok(())
    }
}

/// No allocating Memory::read adapter: caller must copy into the supplied
/// fixed buffer and report exact completion. Native code/locks are never called.
#[cfg(test)]
pub trait CopyMemory {
    fn copy_exact(&mut self, address: u64, out: &mut [u8]) -> bool;
}
#[cfg(test)]
impl<F: FnMut(u64, &mut [u8]) -> bool> CopyMemory for F {
    fn copy_exact(&mut self, address: u64, out: &mut [u8]) -> bool { self(address, out) }
}
#[cfg(test)]
struct Span {
    address: u64,
    width: usize,
    opened: bool,
    bytes: [u8; MAX_TOKEN_BYTES],
}

/// Test-only reference reader for the AcquisitionRead contract.
/// Trace capacity is reserved once BEFORE allocation. Every attempted span
/// occupies one retained entry, so duplicates and failed opens stay charged.
/// This reader must participate in the parent's single acquisition closure;
/// using it with an independently refreshed owner or hour is not sufficient.
#[cfg(test)]
pub struct BoundedReadSet<'a, M: CopyMemory> {
    memory: M,
    budget: &'a mut Budget,
    spans: Vec<Span>,
    span_limit: usize,
    failure: Option<Refusal>,
    closed: bool,
}
#[cfg(test)]
impl<'a, M: CopyMemory> BoundedReadSet<'a, M> {
    pub fn new(memory: M, budget: &'a mut Budget, span_limit: usize) -> Result<Self> {
        budget.allocation(span_limit.checked_mul(size_of::<Span>())
            .ok_or(refusal(RefusalKind::AllocationBudget, "trace capacity"))?)?;
        let mut spans = Vec::new();
        spans.try_reserve_exact(span_limit)
            .map_err(|_| refusal(RefusalKind::AllocationFailed, "trace capacity"))?;
        Ok(Self { memory, budget, spans, span_limit, failure: None, closed: false })
    }
    fn fail<T>(&mut self, error: Refusal) -> Result<T> {
        if self.failure.is_none() { self.failure = Some(error); }
        Err(error)
    }
    /// Close ALL attempted spans, even after a failed opening or closing.
    /// A failed opening has no comparable value but still retains/refuses its
    /// acquisition. A failed seal never allows partial rows to become measured.
    pub fn seal(&mut self) -> Result<()> {
        if self.closed {
            return Err(refusal(RefusalKind::Closed, "readset"));
        }
        self.closed = true;
        for span in &self.spans {
            let mut actual = [0u8; MAX_TOKEN_BYTES];
            let ok = self.memory.copy_exact(span.address, &mut actual[..span.width]);
            let error = if !ok {
                Some(refusal(RefusalKind::Unreadable, "closing read"))
            } else if span.opened && actual[..span.width] != span.bytes[..span.width] {
                Some(refusal(RefusalKind::Torn, "closing bytes"))
            } else { None };
            if self.failure.is_none() { self.failure = error; }
        }
        self.failure.map_or(Ok(()), Err)
    }
}
#[cfg(test)]
impl<M: CopyMemory> AcquisitionRead for BoundedReadSet<'_, M> {
    fn qualify(&mut self, bytes: usize, calls: usize, allocation: usize, spans: usize) -> Result<()> {
        if self.closed { return Err(refusal(RefusalKind::Closed, "readset")); }
        if let Some(error) = self.failure { return Err(error); }
        if self.spans.len().checked_add(spans).map_or(true, |n| n > self.span_limit) {
            return self.fail(refusal(RefusalKind::TraceBound, "complete census spans"));
        }
        if let Err(error) = self.budget.qualify(bytes, calls, allocation) { return self.fail(error); }
        Ok(())
    }
    fn read_exact(&mut self, address: u64, out: &mut [u8]) -> Result<()> {
        if self.closed { return Err(refusal(RefusalKind::Closed, "readset")); }
        if let Some(error) = self.failure { return Err(error); }
        if out.is_empty() { return Ok(()); }
        if out.len() > MAX_TOKEN_BYTES || address == 0 ||
            address.checked_add(out.len() as u64).is_none() {
            return self.fail(refusal(RefusalKind::Address, "read span"));
        }
        if self.spans.len() == self.span_limit {
            return self.fail(refusal(RefusalKind::TraceBound, "retained spans"));
        }
        if let Err(error) = self.budget.opening(out.len()) { return self.fail(error); }
        // Retain address before the opening. Fixed storage; no allocation here.
        self.spans.push(Span { address, width: out.len(), opened: false,
            bytes: [0; MAX_TOKEN_BYTES] });
        let span = self.spans.last_mut().unwrap();
        if !self.memory.copy_exact(address, &mut span.bytes[..span.width]) {
            return self.fail(refusal(RefusalKind::Unreadable, "opening read"));
        }
        span.opened = true;
        out.copy_from_slice(&span.bytes[..span.width]);
        // Repeated/overlapping openings must agree as well as closing spans.
        for prior in &self.spans[..self.spans.len() - 1] {
            let start = address.max(prior.address);
            let end = (address + out.len() as u64).min(prior.address + prior.width as u64);
            if prior.opened && start < end {
                let a = (start - address) as usize;
                let b = (start - prior.address) as usize;
                let n = (end - start) as usize;
                if out[a..a + n] != prior.bytes[b..b + n] {
                    return self.fail(refusal(RefusalKind::Torn, "overlapping openings"));
                }
            }
        }
        Ok(())
    }
    fn reserve_allocation(&mut self, bytes: usize) -> Result<()> {
        if self.closed { return Err(refusal(RefusalKind::Closed, "readset")); }
        if let Some(error) = self.failure { return Err(error); }
        if let Err(error) = self.budget.allocation(bytes) { return self.fail(error); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const BASE: u64 = 0x140000000;
    const VARIANT: u64 = 0x10000000000 + 0x10000;
    const DEFINITION: u64 = 0x10000000000 + 0x18000;
    const ARCHETYPE: u64 = 0x10000000000 + 0x20000;
    const SLOTS: u64 = 0x10000000000 + 0x30000;
    const TABLE: u64 = 0x10000000000 + 0x40000;

    #[derive(Default)]
    struct Mock {
        bytes: BTreeMap<u64, u8>,
        reads: Vec<(u64, usize)>,
        fail_at: Option<usize>,
        change_at: Option<(usize, u64, u8)>,
    }
    impl Mock {
        fn put(&mut self, at: u64, bytes: &[u8]) {
            for (i, &b) in bytes.iter().enumerate() { self.bytes.insert(at + i as u64, b); }
        }
        fn u32(&mut self, at: u64, n: u32) { self.put(at, &n.to_le_bytes()); }
        fn u64(&mut self, at: u64, n: u64) { self.put(at, &n.to_le_bytes()); }
        fn inline(&mut self, at: u64, value: &[u8]) {
            assert!(value.len() < 16);
            let mut h = [0; 32];
            h[..value.len()].copy_from_slice(value);
            h[16..24].copy_from_slice(&(value.len() as u64).to_le_bytes());
            h[24..32].copy_from_slice(&15u64.to_le_bytes());
            self.put(at, &h);
        }
        fn external(&mut self, at: u64, data: u64, value: &[u8]) {
            self.put(at, &[0; 32]);
            self.u64(at, data);
            self.u64(at + 16, value.len() as u64);
            self.u64(at + 24, value.len().max(16) as u64);
            self.put(data, value);
        }
    }
    impl CopyMemory for &mut Mock {
        fn copy_exact(&mut self, at: u64, out: &mut [u8]) -> bool {
            self.reads.push((at, out.len()));
            if let Some((call, address, value)) = self.change_at {
                if self.reads.len() == call { self.bytes.insert(address, value); }
            }
            if self.fail_at == Some(self.reads.len()) { return false; }
            for (i, b) in out.iter_mut().enumerate() {
                let Some(&value) = self.bytes.get(&(at + i as u64)) else { return false; };
                *b = value;
            }
            true
        }
    }
    fn fixture(rows: &[(u32, &str)]) -> Mock {
        let mut m = Mock::default();
        m.u64(VARIANT, BASE + VARIANT_VTABLE_RVA);
        m.u32(VARIANT + 0x18, 42);
        m.u64(VARIANT + 0x3f0, DEFINITION);
        m.u64(DEFINITION, BASE + EQUIPMENT_VTABLE_RVA);
        m.u32(DEFINITION + 8, 42);
        m.u64(DEFINITION + 0x4d8, ARCHETYPE);
        // Synthetic expected type; this fixture is not native type proof.
        m.u64(ARCHETYPE, BASE + EQUIPMENT_VTABLE_RVA);
        m.u32(ARCHETYPE + 8, 99);
        m.inline(ARCHETYPE + 0x18, b"archetype");
        m.u64(ARCHETYPE + 0x70, if rows.is_empty() { 0 } else { SLOTS });
        m.u32(ARCHETYPE + 0x78, rows.len() as u32);
        m.u32(ARCHETYPE + 0x7c, rows.len() as u32);
        m.put(BASE + LEXER_INITIALIZED_RVA, &[1]);
        m.u32(BASE + LEXER_MAX_ID_RVA, 1000);
        m.u64(BASE + LEXER_TABLE_RVA, TABLE);
        m.u32(BASE + LEXER_CAPACITY_RVA, 1001);
        for (i, &(raw, key)) in rows.iter().enumerate() {
            m.u32(SLOTS + i as u64 * 0x50 + 8, raw);
            m.inline(TABLE + raw as u64 * 0x20, key.as_bytes());
        }
        m
    }
    fn owner() -> VariantDefinition {
        VariantDefinition { variant: VARIANT, definition: DEFINITION, equipment_token_raw: 42 }
    }
    fn typed() -> Option<TypedArchetype> {
        Some(TypedArchetype { address: ARCHETYPE,
            vtable_rva: EQUIPMENT_VTABLE_RVA, token_raw: 99 })
    }
    fn budget() -> Budget { Budget::new(1_000_000, 10_000, 1_000_000) }

    #[test]
    fn full_ordered_copied_census_preserves_unassigned_and_sentinel() {
        // There is deliberately no assignment-vector fixture: a slot does not
        // need an assigned module to be observed.
        let mut m = fixture(&[(700, "secondary"), (SENTINEL_TOKEN, "none"), (500, "primary")]);
        let mut b = budget();
        let mut read = BoundedReadSet::new(&mut m, &mut b, 32).unwrap();
        let result = observe(&mut read, BASE, owner(), typed()).unwrap();
        assert_eq!(result.slots.iter().map(|s| (s.source_ordinal, s.token_handle_raw))
            .collect::<Vec<_>>(), vec![(0, 700), (1, 0x165), (2, 500)]);
        assert_eq!(result.slots[1].token, DesignTextObservation::Measured {
            value: "none".into(), utf8_hex: "6e6f6e65".into() });
        assert_eq!(measured_key(&result.archetype_token), "archetype");
        read.seal().unwrap();
    }

    #[test]
    fn observed_zero_count_is_empty_without_lexer() {
        let mut m = fixture(&[]);
        m.put(BASE + LEXER_INITIALIZED_RVA, &[0]);
        let mut b = budget();
        {
            let mut read = BoundedReadSet::new(&mut m, &mut b, 16).unwrap();
            assert!(observe(&mut read, BASE, owner(), typed()).unwrap().slots.is_empty());
            read.seal().unwrap();
        }
        assert!(!m.reads.iter().any(|&(a, _)| a == BASE + LEXER_INITIALIZED_RVA));
    }

    #[test]
    fn null_nonempty_and_negative_count_are_not_empty() {
        for negative in [false, true] {
            let mut m = fixture(&[(500, "primary")]);
            if negative { m.u32(ARCHETYPE + 0x7c, u32::MAX); }
            else { m.u64(ARCHETYPE + 0x70, 0); }
            let mut b = budget();
            let mut read = BoundedReadSet::new(&mut m, &mut b, 20).unwrap();
            assert_eq!(observe(&mut read, BASE, owner(), typed()).unwrap_err().kind,
                RefusalKind::InvalidVector);
        }
    }

    #[test]
    fn missing_typed_archetype_never_guesses_or_reads() {
        let mut m = fixture(&[]);
        let mut b = budget();
        let mut read = BoundedReadSet::new(&mut m, &mut b, 16).unwrap();
        assert_eq!(observe(&mut read, BASE, owner(), None).unwrap_err().kind,
            RefusalKind::MissingTypedArchetype);
        assert!(read.spans.is_empty());
    }

    #[test]
    fn wrong_variant_definition_or_effective_archetype_refuses() {
        for at in [VARIANT + 0x3f0, DEFINITION + 0x4d8, ARCHETYPE] {
            let mut m = fixture(&[]);
            m.u64(at, 0x90000);
            let mut b = budget();
            let mut read = BoundedReadSet::new(&mut m, &mut b, 16).unwrap();
            assert_eq!(observe(&mut read, BASE, owner(), typed()).unwrap_err().kind,
                RefusalKind::InvalidOwner);
        }
    }

    #[test]
    fn lexer_initialized_capacity_and_token_bounds_are_not_fallbacks() {
        for case in 0..5 {
            let mut m = fixture(&[(500, "primary")]);
            match case {
                0 => m.put(BASE + LEXER_INITIALIZED_RVA, &[0]),
                1 => m.u32(BASE + LEXER_CAPACITY_RVA, 1000),
                2 => m.u32(SLOTS + 8, 0),
                3 => m.u32(SLOTS + 8, 1001),
                _ => m.u32(SLOTS + 8, 0x80000000),
            }
            let mut b = budget();
            let mut read = BoundedReadSet::new(&mut m, &mut b, 20).unwrap();
            let kind = observe(&mut read, BASE, owner(), typed()).unwrap_err().kind;
            assert_eq!(kind, if case < 2 { RefusalKind::LexerUnavailable }
                else { RefusalKind::InvalidTokenId });
        }
    }

    #[test]
    fn invalid_utf8_empty_invalid_character_and_oversize_remain_distinct() {
        for (bytes, expected) in [
            (vec![255], RefusalKind::InvalidUtf8),
            (vec![], RefusalKind::InvalidTokenText),
            (b"two words".to_vec(), RefusalKind::InvalidTokenText),
        ] {
            let mut m = fixture(&[(500, "primary")]);
            m.inline(TABLE + 500 * 0x20, &bytes);
            let mut b = budget();
            let mut read = BoundedReadSet::new(&mut m, &mut b, 20).unwrap();
            assert_eq!(observe(&mut read, BASE, owner(), typed()).unwrap_err().kind, expected);
        }
        let mut m = fixture(&[(500, "primary")]);
        m.u64(TABLE + 500 * 0x20 + 16, 161);
        m.u64(TABLE + 500 * 0x20 + 24, 161);
        let mut b = budget();
        let mut read = BoundedReadSet::new(&mut m, &mut b, 20).unwrap();
        assert_eq!(observe(&mut read, BASE, owner(), typed()).unwrap_err().kind,
            RefusalKind::TextBound);
    }

    #[test]
    fn external_text_buffer_participates_in_closing() {
        let mut m = fixture(&[(500, "primary")]);
        m.external(TABLE + 500 * 0x20, 0x10000070000, b"long_primary_slot_key");
        let mut b = budget();
        let mut read = BoundedReadSet::new(&mut m, &mut b, 20).unwrap();
        let result = observe(&mut read, BASE, owner(), typed()).unwrap();
        assert_eq!(measured_key(&result.slots[0].token), "long_primary_slot_key");
        read.memory.u32(0x10000070000, 0x61616161);
        assert_eq!(read.seal().unwrap_err().kind, RefusalKind::Torn);
    }

    #[test]
    fn root_relocation_slot_mutation_and_capacity_change_invalidate_closing() {
        for at in [BASE + LEXER_TABLE_RVA, BASE + LEXER_CAPACITY_RVA,
            SLOTS + 8, ARCHETYPE + 0x70, DEFINITION + 0x4d8] {
            let mut m = fixture(&[(500, "primary")]);
            let mut b = budget();
            let mut read = BoundedReadSet::new(&mut m, &mut b, 20).unwrap();
            let _pending = observe(&mut read, BASE, owner(), typed()).unwrap();
            let original = read.memory.bytes[&at];
            read.memory.bytes.insert(at, original ^ 1);
            assert_eq!(read.seal().unwrap_err().kind, RefusalKind::Torn);
        }
    }

    #[test]
    fn duplicate_raw_id_or_folded_name_is_not_an_authored_binding() {
        for rows in [vec![(500, "primary"), (500, "primary")],
            vec![(500, "primary"), (501, "PRIMARY")]] {
            let mut m = fixture(&rows);
            let mut b = budget();
            let mut read = BoundedReadSet::new(&mut m, &mut b, 24).unwrap();
            assert_eq!(observe(&mut read, BASE, owner(), typed()).unwrap_err().kind,
                RefusalKind::DuplicateIdentity);
        }
    }

    #[test]
    fn failed_open_and_failed_close_keep_charges_and_finish_all_closes() {
        let mut m = Mock::default();
        m.put(0x10000, &[1, 2, 3, 4]);
        m.fail_at = Some(2);
        let mut b = budget();
        {
            let mut read = BoundedReadSet::new(&mut m, &mut b, 2).unwrap();
            let mut out = [0; 4];
            read.read_exact(0x10000, &mut out).unwrap();
            assert_eq!(read.read_exact(0x10000, &mut out).unwrap_err().kind, RefusalKind::Unreadable);
            read.memory.fail_at = Some(3);
            assert_eq!(read.seal().unwrap_err().kind, RefusalKind::Unreadable);
            assert_eq!(read.memory.reads.len(), 4);
        }
        assert_eq!(b.read_bytes_reserved, 16);
        assert_eq!(b.read_calls_reserved, 4);
        assert_eq!(b.allocation_bytes_reserved, 2 * size_of::<Span>());
    }

    #[test]
    fn overlapping_openings_detect_a_change_and_still_close_every_span() {
        let mut m = Mock::default();
        m.put(0x10000, &[1, 2, 3, 4]);
        m.change_at = Some((2, 0x10001, 9));
        let mut b = budget();
        let mut read = BoundedReadSet::new(&mut m, &mut b, 2).unwrap();
        read.read_exact(0x10000, &mut [0; 4]).unwrap();
        assert_eq!(read.read_exact(0x10001, &mut [0; 2]).unwrap_err().kind, RefusalKind::Torn);
        assert_eq!(read.seal().unwrap_err().kind, RefusalKind::Torn);
        assert_eq!(read.memory.reads.len(), 4);
    }

    #[test]
    fn closing_bytes_are_reserved_before_any_opening() {
        let mut m = Mock::default();
        m.put(0x10000, &[1, 2, 3, 4]);
        let mut b = Budget::new(7, 100, 10000);
        {
            let mut read = BoundedReadSet::new(&mut m, &mut b, 1).unwrap();
            assert_eq!(read.read_exact(0x10000, &mut [0; 4]).unwrap_err().kind, RefusalKind::ReadBudget);
            assert!(read.memory.reads.is_empty());
        }
        assert_eq!(b.read_bytes_reserved, 0);
    }

    #[test]
    fn preflight_refuses_before_allocating_rows_or_opening_lexer() {
        let mut m = fixture(&[(500, "primary")]);
        // Enough to open owner/vector, too little for the complete remainder.
        let mut b = Budget::new(201, 1000, 100000);
        let mut read = BoundedReadSet::new(&mut m, &mut b, 20).unwrap();
        assert_eq!(observe(&mut read, BASE, owner(), typed()).unwrap_err().kind, RefusalKind::ReadBudget);
        assert_eq!(read.budget.allocation_bytes_reserved, 20 * size_of::<Span>() + 3 * 9);
        assert!(!read.memory.reads.iter().any(|&(a, _)| a == BASE + LEXER_INITIALIZED_RVA));
    }

    #[test]
    fn allocation_and_trace_limits_refuse_without_native_reads() {
        let mut b = Budget::new(10000, 10000, size_of::<Span>() - 1);
        let mut m = Mock::default();
        assert!(matches!(BoundedReadSet::new(&mut m, &mut b, 1),
            Err(Refusal { kind: RefusalKind::AllocationBudget, .. })));
        assert!(m.reads.is_empty());
        let mut b = budget();
        let mut read = BoundedReadSet::new(&mut m, &mut b, 0).unwrap();
        assert_eq!(read.read_exact(0x10000, &mut [0; 4]).unwrap_err().kind, RefusalKind::TraceBound);
        assert!(read.memory.reads.is_empty());
    }

    #[test]
    fn excessive_row_count_does_not_allocate_rows() {
        let mut m = fixture(&[]);
        m.u32(ARCHETYPE + 0x78, MAX_SLOT_ROWS as u32 + 1);
        m.u32(ARCHETYPE + 0x7c, MAX_SLOT_ROWS as u32 + 1);
        let mut b = budget();
        let mut read = BoundedReadSet::new(&mut m, &mut b, 16).unwrap();
        assert_eq!(observe(&mut read, BASE, owner(), typed()).unwrap_err().kind, RefusalKind::RowBound);
        assert_eq!(read.budget.allocation_bytes_reserved, 16 * size_of::<Span>() + 27);
    }

    #[test]
    fn a_second_acquisition_uses_the_same_meter_without_refund() {
        let mut m = Mock::default();
        m.put(0x10000, &[1; 4]);
        let mut b = Budget::new(8, 2, 10000);
        {
            let mut read = BoundedReadSet::new(&mut m, &mut b, 1).unwrap();
            read.read_exact(0x10000, &mut [0; 4]).unwrap();
            read.seal().unwrap();
            assert_eq!(read.read_exact(0x10000, &mut [0; 4]).unwrap_err().kind, RefusalKind::Closed);
        }
        let mut read = BoundedReadSet::new(&mut m, &mut b, 1).unwrap();
        assert_eq!(read.read_exact(0x10000, &mut [0; 4]).unwrap_err().kind, RefusalKind::ReadBudget);
        assert_eq!(read.memory.reads.len(), 2);
    }
}
