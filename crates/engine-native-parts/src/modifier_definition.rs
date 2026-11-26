
use engine_native_memory::{
    Memory,
    msvc_string::Literal,
    cpdx_array::Array,
    cpdx_lexer,
    read,
    read_slice,
};
use engine_sites::Rva;

pub const TABLE_RVA: Rva = Rva(0x033d_d1d0);
pub const HEADER_LEN: usize = 16;
pub const STRIDE: usize = 0x78;
pub const NAME_KEY: usize = 0x00;
pub const POSTFIX_KEY: usize = 0x20;
pub const ARGUMENT: usize = 0x40;
pub const INDEX: usize = 0x6c;
pub const TOKEN: usize = 0x70;
pub const PRECISION: usize = 0x74;
pub const FLAGS: usize = 0x60;
pub const CATEGORY_MASK: usize = 0x64;
pub const CLAMP_MAXIMUM: u32 = 1 << 4;
pub const FEATURE_SELECTOR: usize = 0x68;
pub const ELIGIBILITY_CALLBACK_RVA: Rva = Rva(0x033d_e580);
pub const ELIGIBILITY_FLAGS_GETTER_RVA: Rva = Rva(0x001b_74a0);
pub const COUNTRY_PREFERRED_TACTIC_FACTOR_RVA: Rva = Rva(0x033e_0d88);
pub const TRADE_FEATURE_FLAGS_RVA: Rva = Rva(0x033d_d698);

#[derive(Default)]
pub struct DefinitionMemo {
    entries: std::cell::RefCell<Vec<(u32, Option<u32>)>>,
}

impl DefinitionMemo {
    pub fn try_with_capacity(keys: usize) -> std::io::Result<Self> {
        Ok(Self {
            entries: std::cell::RefCell::new(engine_allocation::vec(keys)?),
        })
    }

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
        if let Ok(index) = self
            .entries
            .borrow()
            .binary_search_by_key(&key, |&(k, _)| k)
        {
            return Ok(self.entries.borrow()[index].1);
        }
        let flags = read_flags(memory, base, key)?;
        let mut entries = self.entries.borrow_mut();
        match entries.binary_search_by_key(&key, |&(k, _)| k) {
            Ok(index) => Ok(entries[index].1),
            Err(index) => {
                engine_allocation::reserve(&mut entries, 1)
                    .map_err(|_| "modifier definition memo allocation failed")?;
                entries.insert(index, (key, flags));
                Ok(flags)
            }
        }
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

pub fn effective_memo(
    memory: &impl Memory,
    base: usize,
    key: u32,
    raw: i64,
    memo: &DefinitionMemo,
) -> Result<i64, &'static str> {
    effective_observed_memo(memory, base, key, raw, memo).map(|(value, _)| value)
}

pub fn effective_lazy_memo(
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

fn read_flags(memory: &impl Memory, base: usize, key: u32) -> Result<Option<u32>, &'static str> {
    let operands = operands(memory, base, key)?;
    Ok(operands.enabled().then_some(operands.flags))
}

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

    pub fn effective(self, raw: i64) -> i64 {
        apply(self.enabled().then_some(self.flags), raw)
    }
}

pub fn operands(memory: &impl Memory, base: usize, key: u32) -> Result<Operands, &'static str> {
    read_operands(memory, base, key, true)
}

pub fn enabled(memory: &impl Memory, base: usize, key: u32) -> Result<bool, &'static str> {
    Ok(read_operands(memory, base, key, false)?.enabled())
}

fn read_operands(
    memory: &impl Memory,
    base: usize,
    key: u32,
    clamp: bool,
) -> Result<Operands, &'static str> {
    let root = engine_sites::at(base, TABLE_RVA).ok_or("modifier metadata root withheld")?;
    let table = read::<u64>(memory, root)
        .filter(|&p| p != 0)
        .ok_or("modifier metadata table unreadable or null")?;
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
        let callback_address =
            engine_sites::at(base, ELIGIBILITY_CALLBACK_RVA).ok_or("modifier callback withheld")?;
        let callback =
            read::<u64>(memory, callback_address).ok_or("modifier callback unreadable")?;
        if callback == 0 {
            0
        } else {
            if engine_sites::at(base, ELIGIBILITY_FLAGS_GETTER_RVA).map(|p| p as u64)
                != Some(callback)
            {
                return Err("modifier eligibility callback unsupported");
            }
            let flags_address = engine_sites::at(base, TRADE_FEATURE_FLAGS_RVA)
                .ok_or("modifier feature flags withheld")?;
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

pub fn table(memory: &impl Memory, base: usize) -> Result<Array, &'static str> {
    let root = engine_sites::at(base, TABLE_RVA).ok_or("modifier definition table withheld")?;
    let header =
        read::<[u8; HEADER_LEN]>(memory, root).ok_or("modifier definition table unreadable")?;
    Array::from_header(header, STRIDE).ok_or("modifier definition table header invalid")
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub index: u32,
    pub token: u32,
    pub name_key: Span,
    pub postfix_key: Span,
    pub argument: Span,
    pub flags: u32,
    pub category_mask: u32,
    pub feature_selector: u32,
    pub precision: i32,
    pub eligibility: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub text_len: usize,
    pub text_required: usize,
}

use engine_native_memory::walk::Outcome;

impl Walk {
    fn unavailable(reason: &'static str) -> Self {
        Self {
            outcome: Outcome::Unavailable { reason },
            text_len: 0,
            text_required: 0,
        }
    }
}

const ROWS_PER_READ: usize = 16;

fn registered(position: u32, row: &[u8]) -> Result<bool, &'static str> {
    let index = u32::from_le_bytes(row[INDEX..INDEX + 4].try_into().unwrap());
    let token = u32::from_le_bytes(row[TOKEN..TOKEN + 4].try_into().unwrap());
    match (index == position, token != 0) {
        (true, true) => Ok(true),
        (_, false) if index == 0 => Ok(false),
        _ => Err("modifier definition row index or token inconsistent"),
    }
}

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

pub fn walk(
    memory: &impl Memory,
    base: usize,
    rows: &mut Vec<Row>,
    text: &mut Vec<u8>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        engine_native_memory::walk::Buffer::Growing(rows),
        engine_native_memory::walk::Buffer::Growing(text),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    rows: engine_native_memory::walk::Buffer<'_, Row>,
    text: engine_native_memory::walk::Buffer<'_, u8>,
) -> Walk {
    match walk_inner(memory, base, rows, text) {
        Ok(walk) => walk,
        Err(reason) => Walk::unavailable(reason),
    }
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    mut rows: engine_native_memory::walk::Buffer<'_, Row>,
    mut text: engine_native_memory::walk::Buffer<'_, u8>,
) -> Result<Walk, &'static str> {
    let before = table(memory, base)?;
    let eligibility = eligibility(memory, base)?;
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

fn eligibility(memory: &impl Memory, base: usize) -> Result<u8, &'static str> {
    let at = |key: Rva| engine_sites::at(base, key).ok_or("modifier eligibility site withheld");
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

pub fn keys_by_names_at<const N: usize>(
    memory: &impl Memory,
    base: usize,
    names: &[&[u8]; N],
) -> Result<([Option<u32>; N], Array), &'static str> {
    let before = table(memory, base)?;
    let (tokens, lexer) = cpdx_lexer::TokenNames::witnessed(memory, base)?;
    let mut found = [None; N];
    let spelling_len = names.iter().map(|name| name.len()).max().unwrap_or(0);
    let mut spelling = engine_allocation::default_vec::<u8>(spelling_len)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamedKey {
    name: &'static [u8],
    held: Option<(Option<u32>, Array)>,
}

impl NamedKey {
    pub const fn new(name: &'static [u8]) -> Self {
        Self { name, held: None }
    }

    pub fn get(&mut self, memory: &impl Memory, base: usize) -> Result<Option<u32>, &'static str> {
        let current = table(memory, base)?;
        if let Some((key, header)) = self.held
            && header == current
        {
            return Ok(key);
        }
        self.held = None;
        let ([key], header) = keys_by_names_at(memory, base, &[self.name])?;
        self.held = Some((key, header));
        Ok(key)
    }
}

pub fn effective_lazy(
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
        Some(memo) => memo.flags(base, key, || read_flags(memory, base, key)),
        None => read_flags(memory, base, key),
    }
}

#[cfg(test)]
mod tests;
