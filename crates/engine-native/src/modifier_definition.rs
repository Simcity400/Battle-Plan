//! Shared modifier and define metadata table.
//! Feature eligibility is read passively only for the authenticated flags callback.
//!
//! Static registration fills the built-in range, leaving a default-constructed
//! gap before dynamic keys. Gap rows have empty strings, zero flags, zero index
//! and token, and an all-ones category. Dynamic registration increases the count
//! and may replace the data pointer and capacity. A copied table stays current
//! while its data pointer and count are unchanged.
//!
//! Registered rows have an index equal to their position and a nonzero token;
//! the default gap is skipped. `keys_by_names_at` resolves script names through
//! these rows and the lexer's reverse dictionary. It reads every row, so callers
//! resolve once per header.

use crate::{Memory, ccharacter::Literal, cpdx_array::Array, cpdx_lexer, read, read_slice};

/// Modifier-definition table data-pointer global.
pub const TABLE_RVA: usize = 0x0332_ed90;
/// Modifier-definition header: data pointer, signed i32 capacity and signed i32 count.
pub const HEADER_LEN: usize = 16;
/// Modifier-definition metadata row stride.
pub const STRIDE: usize = 0x78;
/// Name localization-key MSVC string; script definitions use their script name.
pub const NAME_KEY: usize = 0x00;
/// Postfix localization-key MSVC string, selected only when nonempty.
pub const POSTFIX_KEY: usize = 0x20;
/// Name-substitution argument MSVC string, used when nonempty.
pub const ARGUMENT: usize = 0x40;
/// Row's own u32 index.
pub const INDEX: usize = 0x6c;
/// Script-name lexer token ID, also used by serialization.
pub const TOKEN: usize = 0x70;
/// Signed i32 display precision, limiting the caller's requested precision.
pub const PRECISION: usize = 0x74;
/// Modifier-definition u32 flags used for value formatting.
pub const FLAGS: usize = 0x60;
/// Modifier-definition u32 category mask used by composition, outside ordinary lookup.
pub const CATEGORY_MASK: usize = 0x64;
/// Modifier-definition clamp-at-one flag.
pub const CLAMP_MAXIMUM: u32 = 1 << 4;
/// Modifier-definition 32-bit feature selector; dynamic keys use zero.
pub const FEATURE_SELECTOR: usize = 0x68;
/// Modifier-eligibility callback-pointer global.
pub const ELIGIBILITY_CALLBACK_RVA: usize = 0x0333_00a0;
/// Authenticated callback supplying the u32 feature-flags word.
pub const ELIGIBILITY_FLAGS_GETTER_RVA: usize = 0x001a_e6a0;
/// Country preferred-tactic weighting-factor global, signed i64 fixed5.
pub const COUNTRY_PREFERRED_TACTIC_FACTOR_RVA: usize = 0x0333_5ee0;
/// Trade-feature flags u32 global.
pub const TRADE_FEATURE_FLAGS_RVA: usize = 0x0332_f248;

/// Passive metadata evaluation for a present accumulator key. No category
/// field participates. The selected callback may be null (feature disabled) or
/// the authenticated feature-flags callback; other targets are not called or
/// guessed. Raw values and presence belong to the caller.
/// The per-walk memo retains eligibility and flags for repeatedly requested keys.
#[derive(Default)]
pub struct DefinitionMemo {
    entries: std::cell::RefCell<Vec<(u32, Option<u32>)>>,
}

impl DefinitionMemo {
    /// A memo that holds `keys` keys before its storage grows.
    pub fn try_with_capacity(keys: usize) -> std::io::Result<Self> {
        Ok(Self {
            entries: std::cell::RefCell::new(crate::allocation::vec(keys)?),
        })
    }

    pub fn with_capacity(keys: usize) -> Self {
        Self {
            entries: std::cell::RefCell::new(Vec::with_capacity(keys)),
        }
    }

    /// Forget every key for the next walk, keeping the storage.
    pub fn clear(&self) {
        self.entries.borrow_mut().clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.entries.borrow().capacity() * size_of::<(u32, Option<u32>)>()
    }

    fn flags(
        &self,
        memory: &impl Memory,
        base: usize,
        key: u32,
    ) -> Result<Option<u32>, &'static str> {
        if let Some((_, flags)) = self.entries.borrow().iter().find(|(k, _)| *k == key) {
            return Ok(*flags);
        }
        let flags = read_flags(memory, base, key)?;
        let mut entries = self.entries.borrow_mut();
        crate::allocation::reserve(&mut entries, 1)
            .map_err(|_| "modifier definition memo allocation failed")?;
        entries.push((key, flags));
        Ok(flags)
    }
}

/// Successful metadata observations for one acquisition pass, keyed by
/// module and modifier key. Failed reads are never retained.
#[derive(Default)]
pub struct PassDefinitions {
    slots: std::cell::RefCell<std::collections::HashMap<(usize, u32), Option<u32>>>,
}

impl PassDefinitions {
    pub fn clear(&mut self) {
        self.slots.get_mut().clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.slots.borrow().capacity() * size_of::<((usize, u32), Option<u32>)>()
    }

    fn flags(
        &self,
        memory: &impl Memory,
        base: usize,
        key: u32,
    ) -> Result<Option<u32>, &'static str> {
        if let Some(flags) = self.slots.borrow().get(&(base, key)) {
            return Ok(*flags);
        }
        let flags = read_flags(memory, base, key)?;
        let mut slots = self.slots.borrow_mut();
        crate::allocation::reserve_map(&mut slots, 1)
            .map_err(|_| "modifier pass memo allocation failed")?;
        slots.insert((base, key), flags);
        Ok(flags)
    }
}

pub fn effective(
    memory: &impl Memory,
    base: usize,
    key: u32,
    raw: i64,
) -> Result<i64, &'static str> {
    effective_lazy(memory, base, key, || Ok(raw))
}

/// `effective` with the per-key metadata read once per walk.
pub fn effective_memo(
    memory: &impl Memory,
    base: usize,
    key: u32,
    raw: i64,
    memo: &DefinitionMemo,
) -> Result<i64, &'static str> {
    effective_observed_memo(memory, base, key, raw, memo).map(|(value, _)| value)
}

/// Requested-key evaluation with pass-local metadata and lazy raw-value access.
pub(crate) fn effective_lazy_memo(
    memory: &impl Memory,
    base: usize,
    key: u32,
    raw: impl FnOnce() -> Result<i64, &'static str>,
    memo: &DefinitionMemo,
) -> Result<i64, &'static str> {
    let flags = memo.flags(memory, base, key)?;
    match flags {
        Some(flags) => Ok(apply(Some(flags), raw()?)),
        None => Ok(0),
    }
}

/// The effective value and the metadata that enabled and clamped it, from
/// the same observation. Disabled keys retain no clamp-flags read.
pub fn effective_observed_memo(
    memory: &impl Memory,
    base: usize,
    key: u32,
    raw: i64,
    memo: &DefinitionMemo,
) -> Result<(i64, Option<u32>), &'static str> {
    let flags = memo.flags(memory, base, key)?;
    Ok((apply(flags, raw), flags))
}

fn apply(flags: Option<u32>, raw: i64) -> i64 {
    match flags {
        Some(flags) if flags & CLAMP_MAXIMUM != 0 => raw.clamp(0, 100000),
        Some(_) => raw,
        None => 0,
    }
}

/// The flags word of an enabled key, None for a disabled one.
fn read_flags(memory: &impl Memory, base: usize, key: u32) -> Result<Option<u32>, &'static str> {
    let operands = operands(memory, base, key)?;
    Ok(operands.enabled().then_some(operands.flags))
}

/// Static per-key metadata. A null authenticated callback supplies no eligible
/// feature bits, and a disabled key does not read its clamp flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Operands {
    pub flags: u32,
    pub selector: u32,
    pub eligibility_flags: u32,
}

impl Operands {
    fn enabled(self) -> bool {
        self.selector == 0 || self.selector & self.eligibility_flags != 0
    }

    /// Effective result for a present raw entry using metadata read in the same pass.
    pub fn effective(self, raw: i64) -> i64 {
        apply(self.enabled().then_some(self.flags), raw)
    }
}

pub fn operands(memory: &impl Memory, base: usize, key: u32) -> Result<Operands, &'static str> {
    read_operands(memory, base, key, true)
}

/// Tests eligibility without reading clamp flags.
pub fn enabled(memory: &impl Memory, base: usize, key: u32) -> Result<bool, &'static str> {
    Ok(read_operands(memory, base, key, false)?.enabled())
}

fn read_operands(
    memory: &impl Memory,
    base: usize,
    key: u32,
    clamp: bool,
) -> Result<Operands, &'static str> {
    let root = base
        .checked_add(TABLE_RVA)
        .ok_or("modifier metadata root overflow")?;
    let table = read::<u64>(memory, root)
        .filter(|&p| p != 0)
        .ok_or("modifier metadata table unreadable or null")?;
    // The native uint key is cast to signed int before its 0x78-byte indexing.
    let displacement = (key as i32 as isize)
        .checked_mul(STRIDE as isize)
        .ok_or("modifier metadata index overflow")?;
    let row = (table as usize)
        .checked_add_signed(displacement)
        .ok_or("modifier metadata row overflow")?;
    let selector_address = row
        .checked_add(FEATURE_SELECTOR)
        .ok_or("modifier selector address overflow")?;
    let selector = read::<u32>(memory, selector_address).ok_or("modifier selector unreadable")?;
    let eligibility_flags = if selector == 0 {
        0
    } else {
        let callback_address = base
            .checked_add(ELIGIBILITY_CALLBACK_RVA)
            .ok_or("modifier callback address overflow")?;
        let callback =
            read::<u64>(memory, callback_address).ok_or("modifier callback unreadable")?;
        if callback == 0 {
            0
        } else {
            if base
                .checked_add(ELIGIBILITY_FLAGS_GETTER_RVA)
                .map(|p| p as u64)
                != Some(callback)
            {
                return Err("modifier eligibility callback unsupported");
            }
            let flags_address = base
                .checked_add(TRADE_FEATURE_FLAGS_RVA)
                .ok_or("modifier feature flags address overflow")?;
            read::<u32>(memory, flags_address).ok_or("modifier feature flags unreadable")?
        }
    };
    let mut operands = Operands {
        flags: 0,
        selector,
        eligibility_flags,
    };
    if clamp && operands.enabled() {
        let address = row
            .checked_add(FLAGS)
            .ok_or("modifier flags address overflow")?;
        operands.flags = read::<u32>(memory, address).ok_or("modifier flags unreadable")?;
    }
    Ok(operands)
}

/// The table header's data pointer and live row count. A copy of the rows
/// stays current while both are unchanged; registration moves or grows the table.
pub fn table(memory: &impl Memory, base: usize) -> Result<Array, &'static str> {
    let root = base
        .checked_add(TABLE_RVA)
        .ok_or("modifier definition table address overflow")?;
    let header =
        read::<[u8; HEADER_LEN]>(memory, root).ok_or("modifier definition table unreadable")?;
    Array::from_header(header, STRIDE).ok_or("modifier definition table header invalid")
}

/// Byte range of one copied string in the walk's text arena.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

/// One registered definition row, with its native fields unchanged.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The row's own index, which equals its table position.
    pub index: u32,
    /// Script-name lexer token id, nonzero for every registered row.
    pub token: u32,
    pub name_key: Span,
    pub postfix_key: Span,
    pub argument: Span,
    pub flags: u32,
    pub category_mask: u32,
    pub feature_selector: u32,
    pub precision: i32,
    /// Process-wide eligibility callback state copied with the table: 0 for null
    /// (every selected feature is ineligible), 1 for the authenticated flags callback.
    /// Combat hovers consult it for a nonzero feature selector.
    pub eligibility: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub text_len: usize,
    /// Text the next attempt needs: exact when every row fit, otherwise a
    /// certified next growth step (`cpdx_lexer::probe_growth_prefix`).
    pub text_required: usize,
}

use crate::walk::Outcome;

impl Walk {
    fn unavailable(reason: &'static str) -> Self {
        Self {
            outcome: Outcome::Unavailable { reason },
            text_len: 0,
            text_required: 0,
        }
    }
}

/// Rows copied per guarded read; the table is one contiguous allocation.
const ROWS_PER_READ: usize = 16;

/// Whether a table position holds a registered definition. Registration
/// writes its index and a nonzero token together; gap rows leave both zero.
/// Any other combination is a layout this reader does not support.
fn registered(position: u32, row: &[u8]) -> Result<bool, &'static str> {
    let index = u32::from_le_bytes(row[INDEX..INDEX + 4].try_into().unwrap());
    let token = u32::from_le_bytes(row[TOKEN..TOKEN + 4].try_into().unwrap());
    match (index == position, token != 0) {
        (true, true) => Ok(true),
        (_, false) if index == 0 => Ok(false),
        _ => Err("modifier definition row index or token inconsistent"),
    }
}

/// Advisory storage demand from the original row and string headers.
/// Payload bytes remain the independent settled-hour acquisition's responsibility.
pub fn storage_demand(memory: &impl Memory, base: usize) -> Result<(usize, usize), &'static str> {
    let before = table(memory, base)?;
    let mut rows = 0_usize;
    let mut text = 0_usize;
    let mut chunk = [0_u8; ROWS_PER_READ * STRIDE];
    let mut first = 0;
    while first < before.count {
        let count = ((before.count - first) as usize).min(ROWS_PER_READ);
        let address = before
            .element(first, STRIDE)
            .ok_or("modifier definition row address overflow")?;
        let bytes = &mut chunk[..count * STRIDE];
        if !read_slice(memory, address, bytes) {
            return Err("modifier definition rows unreadable");
        }
        for (offset, row) in bytes.chunks_exact(STRIDE).enumerate() {
            if !registered(first + offset as u32, row)? {
                continue;
            }
            rows = rows
                .checked_add(1)
                .ok_or("modifier definition row count overflow")?;
            for field in [NAME_KEY, POSTFIX_KEY, ARGUMENT] {
                let literal = Literal::from_header(
                    address + offset * STRIDE + field,
                    row[field..field + 32].try_into().unwrap(),
                )
                .map_err(|_| "modifier definition string invalid")?;
                text = text
                    .checked_add(literal.len)
                    .ok_or("modifier definition text overflow")?;
            }
        }
        first += count as u32;
    }
    if table(memory, base)? != before {
        return Err("modifier definition table changed during sizing");
    }
    Ok((rows, text))
}

fn word(row: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(row[offset..offset + 4].try_into().unwrap())
}

/// Copies every registered definition row in table order. The three strings
/// are copied exactly (MSVC small-string or heap layout); rows and text that do
/// not fit publish a truncated prefix with the remaining registered rows
/// counted. A header change during the copy discards the walk.
pub fn walk(memory: &impl Memory, base: usize, rows: &mut [Row], text: &mut [u8]) -> Walk {
    walk_buffer(
        memory,
        base,
        crate::walk::Buffer::Fixed(rows),
        crate::walk::Buffer::Fixed(text),
    )
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    rows: &mut Vec<Row>,
    text: &mut Vec<u8>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        crate::walk::Buffer::Growing(rows),
        crate::walk::Buffer::Growing(text),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    rows: crate::walk::Buffer<'_, Row>,
    text: crate::walk::Buffer<'_, u8>,
) -> Walk {
    match walk_inner(memory, base, rows, text) {
        Ok(walk) => walk,
        Err(reason) => Walk::unavailable(reason),
    }
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    mut rows: crate::walk::Buffer<'_, Row>,
    mut text: crate::walk::Buffer<'_, u8>,
) -> Result<Walk, &'static str> {
    let before = table(memory, base)?;
    let eligibility = eligibility(memory, base)?;
    // Registration publishes this table extent.
    rows.ensure(before.count as usize)?;
    let mut len = 0;
    let mut unread_rows = 0_u32;
    let mut text_len = 0_usize;
    let mut text_required = None;
    let mut chunk = [0_u8; ROWS_PER_READ * STRIDE];
    let mut first = 0;
    while first < before.count {
        let count = ((before.count - first) as usize).min(ROWS_PER_READ);
        let address = before
            .element(first, STRIDE)
            .ok_or("modifier definition row address overflow")?;
        let bytes = &mut chunk[..count * STRIDE];
        if !read_slice(memory, address, bytes) {
            return Err("modifier definition rows unreadable");
        }
        for (offset, row) in bytes.chunks_exact(STRIDE).enumerate() {
            let position = first + offset as u32;
            if !registered(position, row)? {
                continue;
            }
            if unread_rows != 0 || len == rows.len() {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("modifier definition row count overflow")?;
                continue;
            }
            let row_address = address + offset * STRIDE;
            let strings = [NAME_KEY, POSTFIX_KEY, ARGUMENT].map(|field| {
                Literal::from_header(
                    row_address + field,
                    row[field..field + 32].try_into().unwrap(),
                )
                .map_err(|_| "modifier definition string invalid")
            });
            let [name, postfix, argument] = strings;
            let (name, postfix, argument) = (name?, postfix?, argument?);
            Literal::validate(memory, [&name, &postfix, &argument])
                .map_err(|_| "modifier definition string terminator unreadable or invalid")?;
            let needed = name
                .len
                .checked_add(postfix.len)
                .and_then(|n| n.checked_add(argument.len))
                .ok_or("modifier definition text overflow")?;
            let end = text_len
                .checked_add(needed)
                .ok_or("modifier definition text overflow")?;
            text.ensure(end)?;
            if end > text.len() {
                // Certify only the next growth step from the first string that
                // does not fit, as the lexical dictionary does.
                let mut used = text_len;
                let mut required = used;
                for literal in [&name, &postfix, &argument] {
                    if used + literal.len > text.len() {
                        required = cpdx_lexer::probe_growth_prefix(
                            memory,
                            literal.source(),
                            literal.len,
                            used,
                            text.len(),
                        )
                        .map_err(|_| "modifier definition string bytes unreadable")?;
                        break;
                    }
                    used += literal.len;
                }
                text_required = Some(required);
                unread_rows = 1;
                continue;
            }
            Literal::copy_all(
                memory,
                [&name, &postfix, &argument],
                &mut text[text_len..end],
            )
            .map_err(|_| "modifier definition string bytes unreadable")?;
            let span = |start: usize, len: usize| Span { start, len };
            rows[len] = Row {
                index: position,
                token: word(row, TOKEN),
                name_key: span(text_len, name.len),
                postfix_key: span(text_len + name.len, postfix.len),
                argument: span(text_len + name.len + postfix.len, argument.len),
                flags: word(row, FLAGS),
                category_mask: word(row, CATEGORY_MASK),
                feature_selector: word(row, FEATURE_SELECTOR),
                precision: word(row, PRECISION) as i32,
                eligibility,
            };
            len += 1;
            text_len = end;
        }
        first += count as u32;
    }
    if table(memory, base)? != before {
        return Err("modifier definition table changed during copy");
    }
    Ok(Walk {
        outcome: if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        },
        text_len,
        text_required: text_required.unwrap_or(text_len),
    })
}

/// Eligibility callback state: 0 for null, 1 for the authenticated callback
/// supplying feature flags. Other callbacks are not evaluated.
fn eligibility(memory: &impl Memory, base: usize) -> Result<u8, &'static str> {
    let at = |rva: usize| base.checked_add(rva).ok_or("module address overflow");
    let callback = read::<u64>(memory, at(ELIGIBILITY_CALLBACK_RVA)?)
        .ok_or("modifier eligibility callback unreadable")?;
    if callback == 0 {
        Ok(0)
    } else if at(ELIGIBILITY_FLAGS_GETTER_RVA)? as u64 == callback {
        Ok(1)
    } else {
        Err("modifier eligibility callback unsupported")
    }
}

/// Modifier indices resolved from script names through this table and the
/// lexer's reverse dictionary; `None` when no row carries a name. An index
/// registered twice keeps only its later name, so earlier aliases are absent.
/// Rows are copied in batches and each registered script name is read once.
/// Returns the selecting table header, allowing reuse while [`table`] reports
/// the same data pointer and count.
pub fn keys_by_names_at<const N: usize>(
    memory: &impl Memory,
    base: usize,
    names: &[&[u8]; N],
) -> Result<([Option<u32>; N], Array), &'static str> {
    let before = table(memory, base)?;
    let (tokens, lexer) = cpdx_lexer::TokenNames::witnessed(memory, base)?;
    let mut found = [None; N];
    let spelling_len = names.iter().map(|name| name.len()).max().unwrap_or(0);
    let mut spelling = crate::allocation::default_vec::<u8>(spelling_len)
        .map_err(|_| "modifier script name allocation failed")?;
    let mut chunk = [0_u8; ROWS_PER_READ * STRIDE];
    let mut first = 0;
    while first < before.count {
        let count = ((before.count - first) as usize).min(ROWS_PER_READ);
        let address = before
            .element(first, STRIDE)
            .ok_or("modifier definition row address overflow")?;
        let bytes = &mut chunk[..count * STRIDE];
        if !read_slice(memory, address, bytes) {
            return Err("modifier definition row unreadable");
        }
        for (offset, row) in bytes.chunks_exact(STRIDE).enumerate() {
            let position = first + offset as u32;
            if !registered(position, row)? {
                continue;
            }
            let text = tokens.text(memory, word(row, TOKEN))?;
            if !names.iter().any(|name| name.len() == text.len()) {
                continue;
            }
            text.copy(memory, &mut spelling[..text.len()])?;
            for (name, slot) in names.iter().zip(found.iter_mut()) {
                if *name == &spelling[..text.len()] {
                    if slot.is_some() {
                        return Err("modifier script name registered at more than one index");
                    }
                    *slot = Some(position);
                }
            }
        }
        first += count as u32;
    }
    cpdx_lexer::unchanged(memory, base, lexer)?;
    if table(memory, base)? != before {
        return Err("modifier definition table changed during lookup");
    }
    Ok((found, before))
}

/// A present entry's raw value is read only after feature eligibility.
pub(crate) fn effective_lazy(
    memory: &impl Memory,
    base: usize,
    key: u32,
    raw: impl FnOnce() -> Result<i64, &'static str>,
) -> Result<i64, &'static str> {
    let flags = effective_flags(memory, base, key)?;
    match flags {
        Some(flags) => Ok(apply(Some(flags), raw()?)),
        None => Ok(0),
    }
}

fn effective_flags(
    memory: &impl Memory,
    base: usize,
    key: u32,
) -> Result<Option<u32>, &'static str> {
    match memory.modifier_definitions() {
        Some(memo) => memo.flags(memory, base, key),
        None => read_flags(memory, base, key),
    }
}

#[cfg(test)]
mod tests;
