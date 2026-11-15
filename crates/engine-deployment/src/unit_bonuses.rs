//! A country's persistent unit-bonus list, embedded in its deployment status
//! at [`cdeployment_status::UNIT_BONUSES`]. The unit-bonus effect, doctrine
//! rewards and milestones, and special-project rewards each append one entry;
//! removing a source erases its entries. The game rebuilds the country's local
//! sub-unit clones from the list, applying its entries in stored order.
//!
//! An entry holds its source (kind, id token, signed number and label) and a
//! payload of exact sub-unit targets and sub-unit category targets. Each
//! target owns a bonus definition, whose dense stat words and combat width the
//! clone rebuild adds and whose battalion grants it merges. The definition's
//! requirements, terrain and special adjusters, priorities, sprite, transport
//! restriction and own-fuel multiplier are not read: the clones' effective
//! requirements are CountryLocalSubunits', and the rebuild applies none of the
//! others.
//!
//! The walk reads every canonical country. Its deployment status is
//! authenticated by vtable and owner backlink, each entry and payload by
//! vtable, and each owned bonus definition by the sub-unit definition vtable.
//! Any failure makes the whole walk unavailable: a partial list cannot be told
//! from a shorter one. An entry is stored whole or not at all; one that does
//! not fit a fixed output is counted unread with its demand, so a later
//! capture grows off-thread. Nothing certifies the list between reads.

use crate::{
    Memory, ccountry, cdeployment_status, cequipment_variant,
    cpdx_array::Array,
    creferenced_division_template::Span,
    csub_unit_definition, msvc_string, read,
    walk::{Buffer, Outcome},
};
use engine_sites::Rva;

/// List entry stride.
pub const STRIDE: usize = 0x70;
/// Entry vtable.
pub const VTABLE_RVA: Rva = Rva(0x029d_c408);
/// The entry's embedded bonus payload.
pub const PAYLOAD: usize = 0x08;
/// The payload's vtable.
pub const PAYLOAD_VTABLE_RVA: Rva = Rva(0x0285_e858);
/// Payload CPdxArray of exact sub-unit targets: pointers to owned bonus
/// definitions, whose token names the target sub-unit.
pub const EXACT_TARGETS: usize = 0x08;
pub const EXACT_TARGET_STRIDE: usize = 0x08;
/// Payload CPdxArray of sub-unit category targets: the category pointer, then
/// the owned bonus definition pointer at [`CATEGORY_TARGET_BONUS`].
pub const CATEGORY_TARGETS: usize = 0x20;
pub const CATEGORY_TARGET_STRIDE: usize = 0x10;
pub const CATEGORY_TARGET_BONUS: usize = 0x08;
/// Source kind, u32, below [`SOURCE_KINDS`]: none, special project, effect,
/// grand doctrine, sub-doctrine, doctrine reward and doctrine milestone.
pub const SOURCE_KIND: usize = 0x40;
pub const SOURCE_KINDS: u32 = 7;
/// Source id token, u32.
pub const SOURCE_ID: usize = 0x44;
/// Source number, signed i32: a doctrine reward's ordinal or a milestone's
/// track index, otherwise zero.
pub const SOURCE_NUMBER: usize = 0x48;
/// Source label MSVC string, the unit-bonus effect's name; it ends the entry.
pub const LABEL: usize = 0x50;
/// A sub-unit category's token, u32.
pub const CATEGORY_TOKEN: usize = 0x54;
/// An owned bonus definition's battalion-grant CPdxArray.
pub const BATTALION_GRANTS: usize = 0x88;
pub const BATTALION_GRANT_STRIDE: usize = 0x30;
/// A grant's recipient sub-unit category pointer.
pub const GRANT_CATEGORY: usize = 0x08;
/// A grant's sparse stat CPdxArray: a u32 stat selector, then its signed
/// fixed5 amount at [`GRANT_STAT_AMOUNT`].
pub const GRANT_STATS: usize = 0x10;
pub const GRANT_STAT_STRIDE: usize = 0x10;
pub const GRANT_STAT_AMOUNT: usize = 0x08;
/// A grant's add byte.
pub const GRANT_ADD: usize = 0x28;
/// A grant's display-as-percentage byte.
pub const GRANT_DISPLAY_AS_PERCENTAGE: usize = 0x29;
/// An owned bonus definition's dense stat words: one signed fixed5 word per
/// stat selector, in the executable's selector order.
pub const STATS: usize = 0xa0;
/// An owned bonus definition from its vtable through its last stat word.
const DEFINITION_SPAN: usize = STATS + cequipment_variant::STAT_COUNT * 8;

/// One list entry, in stored order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Raw CCountry id of the owning deployment status.
    pub country: u32,
    /// Position in the country's list.
    pub ordinal: u32,
    /// Below [`SOURCE_KINDS`].
    pub source_kind: u8,
    pub source_id: u32,
    pub source_number: i32,
    /// Label bytes in the label arena.
    pub label: Span,
    /// The entry's targets in the target arena: every exact target, then every
    /// category target, each in array order.
    pub targets: Span,
}

/// One target of an entry.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// False for an exact sub-unit target, true for a sub-unit category target.
    pub category: bool,
    /// The target sub-unit's token, or the category's.
    pub token: u32,
    /// Signed fixed5 combat width.
    pub combat_width_raw: i64,
    /// The nonzero dense stat words in the stat arena, in selector order.
    pub stats: Span,
    /// Battalion grants in the grant arena, in stored order.
    pub grants: Span,
}

/// One battalion grant of a target.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Grant {
    /// The recipient sub-unit category's token.
    pub category_token: u32,
    /// Raw add byte.
    pub add: u8,
    /// Raw display-as-percentage byte.
    pub display_as_percentage: u8,
    /// The grant's sparse stats in the stat arena, in stored order with zero
    /// amounts and repeated selectors.
    pub stats: Span,
}

/// One stat selector and its signed fixed5 value.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub selector: u32,
    pub value_raw: i64,
}

/// Lengths of the five outputs.
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
    /// What the outputs hold; `rows` is the outcome's length.
    pub written: Counts,
    /// What every entry reached needs, stored or not, including a failed
    /// walk's entries before its failure.
    pub required: Counts,
}

/// The walk's outputs: the entries and the arenas their spans index.
pub struct Buffers<'a> {
    pub rows: Buffer<'a, Row>,
    pub targets: Buffer<'a, Target>,
    pub grants: Buffer<'a, Grant>,
    pub stats: Buffer<'a, Stat>,
    pub labels: Buffer<'a, u8>,
}

impl<'a> Buffers<'a> {
    /// Worker-owned outputs, grown from the headers the walk reads.
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

    /// No storage: the walk only counts demand.
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

/// One output: the values stored so far and the demand of every value reached.
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

    /// Stores `value` when it fits, growing a worker-owned output; counts its
    /// demand either way.
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
    /// Stores a validated string's bytes when they fit. A worker-owned arena
    /// grows only as the bytes are copied.
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
    /// Entries that did not fit.
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

    /// Withdraws a partly stored entry and counts it unread.
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

/// The expected vtable words, at the module base.
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

/// Every canonical country's list, in country order and stored order.
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

/// One target's owned bonus definition. A category target carries its
/// category's token; an exact target is named by the definition's own.
/// Returns whether everything it stored fit.
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
