
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    msvc_string,
    read,
    walk::{Buffer, Outcome},
};
use engine_native::{ccountry, cequipment_variant, creferenced_division_template::Span};
use crate::cdeployment_status;
use engine_native_parts::csub_unit_definition;
use engine_sites::Rva;

pub const STRIDE: usize = 0x70;
pub const VTABLE_RVA: Rva = Rva(0x029d_c408);
pub const PAYLOAD: usize = 0x08;
pub const PAYLOAD_VTABLE_RVA: Rva = Rva(0x0285_e858);
pub const EXACT_TARGETS: usize = 0x08;
pub const EXACT_TARGET_STRIDE: usize = 0x08;
pub const CATEGORY_TARGETS: usize = 0x20;
pub const CATEGORY_TARGET_STRIDE: usize = 0x10;
pub const CATEGORY_TARGET_BONUS: usize = 0x08;
pub const SOURCE_KIND: usize = 0x40;
pub const SOURCE_KINDS: u32 = 7;
pub const SOURCE_ID: usize = 0x44;
pub const SOURCE_NUMBER: usize = 0x48;
pub const LABEL: usize = 0x50;
pub const CATEGORY_TOKEN: usize = 0x54;
pub const BATTALION_GRANTS: usize = 0x88;
pub const BATTALION_GRANT_STRIDE: usize = 0x30;
pub const GRANT_CATEGORY: usize = 0x08;
pub const GRANT_STATS: usize = 0x10;
pub const GRANT_STAT_STRIDE: usize = 0x10;
pub const GRANT_STAT_AMOUNT: usize = 0x08;
pub const GRANT_ADD: usize = 0x28;
pub const GRANT_DISPLAY_AS_PERCENTAGE: usize = 0x29;
pub const STATS: usize = 0xa0;
const DEFINITION_SPAN: usize = STATS + cequipment_variant::STAT_COUNT * 8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub ordinal: u32,
    pub source_kind: u8,
    pub source_id: u32,
    pub source_number: i32,
    pub label: Span,
    pub targets: Span,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub category: bool,
    pub token: u32,
    pub combat_width_raw: i64,
    pub stats: Span,
    pub grants: Span,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Grant {
    pub category_token: u32,
    pub add: u8,
    pub display_as_percentage: u8,
    pub stats: Span,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub selector: u32,
    pub value_raw: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub rows: usize,
    pub targets: usize,
    pub grants: usize,
    pub stats: usize,
    pub label_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub written: Counts,
    pub required: Counts,
}

pub struct Buffers<'a> {
    pub rows: Buffer<'a, Row>,
    pub targets: Buffer<'a, Target>,
    pub grants: Buffer<'a, Grant>,
    pub stats: Buffer<'a, Stat>,
    pub labels: Buffer<'a, u8>,
}

impl<'a> Buffers<'a> {
    pub fn growing(
        rows: &'a mut Vec<Row>,
        targets: &'a mut Vec<Target>,
        grants: &'a mut Vec<Grant>,
        stats: &'a mut Vec<Stat>,
        labels: &'a mut Vec<u8>,
    ) -> Self {
        Self {
            rows: Buffer::Growing(rows),
            targets: Buffer::Growing(targets),
            grants: Buffer::Growing(grants),
            stats: Buffer::Growing(stats),
            labels: Buffer::Growing(labels),
        }
    }

    pub fn demand() -> Self {
        Self {
            rows: Buffer::Count,
            targets: Buffer::Count,
            grants: Buffer::Count,
            stats: Buffer::Count,
            labels: Buffer::Count,
        }
    }
}

const OVERFLOW: &str = "unit bonus count overflow";

struct Sink<'a, T> {
    values: Buffer<'a, T>,
    len: usize,
    required: usize,
}

impl<'a, T: Default> Sink<'a, T> {
    fn new(values: Buffer<'a, T>) -> Self {
        Self {
            values,
            len: 0,
            required: 0,
        }
    }

    fn push(&mut self, value: T) -> Result<bool, &'static str> {
        self.required = self.required.checked_add(1).ok_or(OVERFLOW)?;
        let end = self.len.checked_add(1).ok_or(OVERFLOW)?;
        if !self.values.ensure(end)? {
            return Ok(false);
        }
        self.values[self.len] = value;
        self.len = end;
        Ok(true)
    }
}

impl Sink<'_, u8> {
    fn push_text(
        &mut self,
        memory: &impl Memory,
        text: &msvc_string::Descriptor,
    ) -> Result<bool, &'static str> {
        self.required = self.required.checked_add(text.len).ok_or(OVERFLOW)?;
        let end = self.len.checked_add(text.len).ok_or(OVERFLOW)?;
        if !self.values.is_growing() && end > self.values.len() {
            return Ok(false);
        }
        text.copy_buffer(memory, &mut self.values, self.len)?;
        self.len = end;
        Ok(true)
    }
}

struct Output<'a> {
    rows: Sink<'a, Row>,
    targets: Sink<'a, Target>,
    grants: Sink<'a, Grant>,
    stats: Sink<'a, Stat>,
    labels: Sink<'a, u8>,
    missing: u32,
}

impl Output<'_> {
    fn marks(&self) -> [usize; 5] {
        [
            self.rows.len,
            self.targets.len,
            self.grants.len,
            self.stats.len,
            self.labels.len,
        ]
    }

    fn withdraw(&mut self, marks: [usize; 5]) -> Result<(), &'static str> {
        [
            self.rows.len,
            self.targets.len,
            self.grants.len,
            self.stats.len,
            self.labels.len,
        ] = marks;
        self.missing = self.missing.checked_add(1).ok_or(OVERFLOW)?;
        Ok(())
    }

    fn counts(&self, count: impl Fn(usize, usize) -> usize) -> Counts {
        Counts {
            rows: count(self.rows.len, self.rows.required),
            targets: count(self.targets.len, self.targets.required),
            grants: count(self.grants.len, self.grants.required),
            stats: count(self.stats.len, self.stats.required),
            label_bytes: count(self.labels.len, self.labels.required),
        }
    }
}

struct Identity {
    entry: u64,
    payload: u64,
    definition: u64,
}

fn word(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn dword(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn header(bytes: &[u8], at: usize, stride: usize) -> Option<Array> {
    Array::from_header(bytes[at..at + 16].try_into().unwrap(), stride)
}

fn pointer(word: u64) -> Option<usize> {
    usize::try_from(word).ok().filter(|pointer| *pointer != 0)
}

pub fn walk(memory: &impl Memory, base: usize, game_state: usize, buffers: Buffers<'_>) -> Walk {
    let mut output = Output {
        rows: Sink::new(buffers.rows),
        targets: Sink::new(buffers.targets),
        grants: Sink::new(buffers.grants),
        stats: Sink::new(buffers.stats),
        labels: Sink::new(buffers.labels),
        missing: 0,
    };
    let captured = (|| {
        let vtable = |key: Rva| {
            engine_sites::at(base, key)
                .map(|vtable| vtable as u64)
                .ok_or("unit bonus module address overflow")
        };
        let identity = Identity {
            entry: vtable(VTABLE_RVA)?,
            payload: vtable(PAYLOAD_VTABLE_RVA)?,
            definition: vtable(csub_unit_definition::VTABLE_RVA)?,
        };
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let list = cdeployment_status::manager(memory, base, country)?
                .checked_add(cdeployment_status::UNIT_BONUSES)
                .and_then(|at| Array::read(memory, at, STRIDE))
                .ok_or("unit bonus list invalid")?;
            for ordinal in 0..list.count {
                read_entry(memory, &identity, country.id, list, ordinal, &mut output)?;
            }
        }
        Ok(())
    })();
    Walk {
        outcome: match captured {
            Ok(()) if output.missing == 0 => Outcome::Rows {
                len: output.rows.len,
            },
            Ok(()) => Outcome::Truncated {
                len: output.rows.len,
                unread_rows: output.missing,
            },
            Err(reason) => Outcome::Unavailable { reason },
        },
        written: output.counts(|len, _| len),
        required: output.counts(|_, required| required),
    }
}

fn read_entry(
    memory: &impl Memory,
    identity: &Identity,
    country: u32,
    list: Array,
    ordinal: u32,
    output: &mut Output<'_>,
) -> Result<(), &'static str> {
    let at = list
        .element(ordinal, STRIDE)
        .ok_or("unit bonus index invalid")?;
    let entry = read::<[u8; STRIDE]>(memory, at).ok_or("unit bonus entry unreadable")?;
    if word(&entry, 0) != identity.entry {
        return Err("unit bonus entry identity mismatch");
    }
    if word(&entry, PAYLOAD) != identity.payload {
        return Err("unit bonus payload identity mismatch");
    }
    let source_kind = dword(&entry, SOURCE_KIND);
    if source_kind >= SOURCE_KINDS {
        return Err("unit bonus source kind unknown");
    }
    let exact = header(&entry, PAYLOAD + EXACT_TARGETS, EXACT_TARGET_STRIDE)
        .ok_or("unit bonus exact target array invalid")?;
    let categories = header(&entry, PAYLOAD + CATEGORY_TARGETS, CATEGORY_TARGET_STRIDE)
        .ok_or("unit bonus category target array invalid")?;
    let label = msvc_string::Descriptor::from_header(
        memory,
        at.checked_add(LABEL).ok_or("unit bonus address overflow")?,
        entry[LABEL..LABEL + msvc_string::STRIDE]
            .try_into()
            .unwrap(),
    )?;
    let marks = output.marks();
    let mut fits = true;
    let first = output.targets.len;
    for index in 0..exact.count {
        let definition = exact
            .element(index, EXACT_TARGET_STRIDE)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(pointer)
            .ok_or("unit bonus exact target unreadable or null")?;
        fits &= read_target(memory, identity, None, definition, output)?;
    }
    for index in 0..categories.count {
        let pair = categories
            .element(index, CATEGORY_TARGET_STRIDE)
            .and_then(|at| read::<[u8; CATEGORY_TARGET_STRIDE]>(memory, at))
            .ok_or("unit bonus category target unreadable")?;
        let (Some(category), Some(definition)) = (
            pointer(word(&pair, 0)),
            pointer(word(&pair, CATEGORY_TARGET_BONUS)),
        ) else {
            return Err("unit bonus category target null");
        };
        let token = category_token(memory, category)?;
        fits &= read_target(memory, identity, Some(token), definition, output)?;
    }
    let targets = Span {
        start: first,
        len: output.targets.len - first,
    };
    let label_start = output.labels.len;
    fits &= output.labels.push_text(memory, &label)?;
    fits &= output.rows.push(Row {
        country,
        ordinal,
        source_kind: source_kind as u8,
        source_id: dword(&entry, SOURCE_ID),
        source_number: dword(&entry, SOURCE_NUMBER) as i32,
        label: Span {
            start: label_start,
            len: label.len,
        },
        targets,
    })?;
    if !fits {
        output.withdraw(marks)?;
    }
    Ok(())
}

fn read_target(
    memory: &impl Memory,
    identity: &Identity,
    category: Option<u32>,
    definition: usize,
    output: &mut Output<'_>,
) -> Result<bool, &'static str> {
    let bytes = read::<[u8; DEFINITION_SPAN]>(memory, definition)
        .ok_or("unit bonus definition unreadable")?;
    let own_token = dword(&bytes, csub_unit_definition::TOKEN);
    if word(&bytes, 0) != identity.definition || (category.is_none() && own_token == 0) {
        return Err("unit bonus definition identity mismatch");
    }
    let grants = header(&bytes, BATTALION_GRANTS, BATTALION_GRANT_STRIDE)
        .ok_or("unit bonus battalion grant array invalid")?;
    let mut fits = true;
    let first_stat = output.stats.len;
    for selector in 0..cequipment_variant::STAT_COUNT {
        let value_raw = word(&bytes, STATS + selector * 8) as i64;
        if value_raw != 0 {
            fits &= output.stats.push(Stat {
                selector: selector as u32,
                value_raw,
            })?;
        }
    }
    let stats = Span {
        start: first_stat,
        len: output.stats.len - first_stat,
    };
    let first_grant = output.grants.len;
    for index in 0..grants.count {
        let grant = grants
            .element(index, BATTALION_GRANT_STRIDE)
            .and_then(|at| read::<[u8; BATTALION_GRANT_STRIDE]>(memory, at))
            .ok_or("unit bonus battalion grant unreadable")?;
        let recipient = pointer(word(&grant, GRANT_CATEGORY))
            .ok_or("unit bonus battalion grant category null")?;
        let sparse = header(&grant, GRANT_STATS, GRANT_STAT_STRIDE)
            .ok_or("unit bonus battalion grant stat array invalid")?;
        let start = output.stats.len;
        for entry in 0..sparse.count {
            let stat = sparse
                .element(entry, GRANT_STAT_STRIDE)
                .and_then(|at| read::<[u8; GRANT_STAT_STRIDE]>(memory, at))
                .ok_or("unit bonus battalion grant stat unreadable")?;
            let selector = dword(&stat, 0);
            if selector as usize >= cequipment_variant::STAT_COUNT {
                return Err("unit bonus battalion grant stat selector outside the stat block");
            }
            fits &= output.stats.push(Stat {
                selector,
                value_raw: word(&stat, GRANT_STAT_AMOUNT) as i64,
            })?;
        }
        fits &= output.grants.push(Grant {
            category_token: category_token(memory, recipient)?,
            add: grant[GRANT_ADD],
            display_as_percentage: grant[GRANT_DISPLAY_AS_PERCENTAGE],
            stats: Span {
                start,
                len: output.stats.len - start,
            },
        })?;
    }
    fits &= output.targets.push(Target {
        category: category.is_some(),
        token: category.unwrap_or(own_token),
        combat_width_raw: word(&bytes, csub_unit_definition::COMBAT_WIDTH) as i64,
        stats,
        grants: Span {
            start: first_grant,
            len: output.grants.len - first_grant,
        },
    })?;
    Ok(fits)
}

fn category_token(memory: &impl Memory, category: usize) -> Result<u32, &'static str> {
    category
        .checked_add(CATEGORY_TOKEN)
        .and_then(|at| read::<u32>(memory, at))
        .filter(|token| *token != 0)
        .ok_or("sub-unit category token unreadable or null")
}

#[cfg(test)]
mod tests;
