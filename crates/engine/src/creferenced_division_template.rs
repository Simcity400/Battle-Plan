//! CReferencedDivisionTemplate identity and retained model preferences.

/// CReferencedDivisionTemplate vtable; old offsets.rs:639, pinned RTTI COL at RVA2CB8218 confirms the actual class name.
pub const VTABLE_RVA: usize = 0x0293_8f70;
/// CReferencedDivisionTemplate reference type; old offsets.rs:640 and template reader.
pub const REFERENCE_TYPE: u32 = 52;
/// CReferencedDivisionTemplate u32 type word; old offsets.rs:641 and template reader.
pub const TYPE: usize = 0x08;
/// Skips the fog reveal of this template's divisions: `GetDivisionRevealSkipFlag@140C79AA0`
/// reads CArmy+0x3B8's byte here for `radar_group::fog_consumer@141606390`.
pub const REVEAL_SKIP: usize = 0x234;
/// CReferencedDivisionTemplate u32 id; old offsets.rs:642 and template reader.
pub const ID: usize = 0x0c;
/// CReferencedDivisionTemplate contained composition storage; old UZ func_140c6f080.decompile.c returns template+18, no RTTI base there.
pub const COMPOSITION: usize = 0x18;
/// CReferencedDivisionTemplate name MSVC string; old recorder_host/native_design_catalog.rs:492 receives registry primary pointer.
pub const NAME: usize = 0x20;
/// CReferencedDivisionTemplate grid dimensions, columns low16/rows high16; old native_design_catalog.rs:493-495.
pub const GRID_DIMENSIONS: usize = 0x60;
/// CReferencedDivisionTemplate regiment CSubUnitDefinition pointer-vector data; old native_design_catalog.rs:500.
pub const REGIMENT_DATA: usize = 0x68;
/// CReferencedDivisionTemplate regiment u32 count; old native_design_catalog.rs:500, row stride8.
pub const REGIMENT_COUNT: usize = 0x74;
/// CReferencedDivisionTemplate support CSubUnitDefinition pointer-vector data; old native_design_catalog.rs:503.
pub const SUPPORT_DATA: usize = 0x88;
/// CReferencedDivisionTemplate support u32 count; old native_design_catalog.rs:501-503, row stride8.
pub const SUPPORT_COUNT: usize = 0x94;
/// CReferencedDivisionTemplate regimental-support definition pointer-vector data; old native_design_catalog.rs:505, primary pointer.
pub const REGIMENTAL_SUPPORT_DATA: usize = 0xa8;
/// CReferencedDivisionTemplate regimental-support u32 count; old native_design_catalog.rs:505, row stride8 not contained composition pairs.
pub const REGIMENTAL_SUPPORT_COUNT: usize = 0xb4;
/// CReferencedDivisionTemplate excluded CEquipmentVariant pointer-vector data; division_presentation_inputs.rs:105-132.
pub const EXCLUDED_VARIANT_DATA: usize = 0x140;
/// CReferencedDivisionTemplate exclusion u32 capacity; division_presentation_inputs.rs:107.
pub const EXCLUDED_VARIANT_CAPACITY: usize = 0x148;
/// CReferencedDivisionTemplate exclusion u32 count; division_presentation_inputs.rs:108.
pub const EXCLUDED_VARIANT_COUNT: usize = 0x14c;
/// CReferencedDivisionTemplate u32 priority; old native_design_catalog.rs:507.
pub const PRIORITY: usize = 0x1a4;
/// CReferencedDivisionTemplate lock-regime snapshot byte; old native_design_catalog.rs:507, no boolean collapse.
pub const LOCK_REGIME: usize = 0x1cc;
/// CReferencedDivisionTemplate token289A country operand i32; old native_design_catalog/native_inputs.rs:37 and template_country_inputs.
pub const COUNTRY_OPERAND_289A: usize = 0x1ec;
/// CReferencedDivisionTemplate token3ABE country operand i32; old native_design_catalog/native_inputs.rs:38 and template_country_inputs.
pub const COUNTRY_OPERAND_3ABE: usize = 0x1f4;
/// CReferencedDivisionTemplate raw model-category u64; division_presentation_inputs.rs:151, meaningful only when presence byte is set.
pub const MODEL_CATEGORY: usize = 0x200;
/// CReferencedDivisionTemplate category-presence byte; division_presentation_inputs.rs:147.
pub const MODEL_CATEGORY_PRESENT: usize = 0x208;
/// CReferencedDivisionTemplate MSVC entity-name string; division_presentation_inputs.rs:144, engine-owned bytes preserved.
pub const MODEL_ENTITY: usize = 0x210;
/// CReferencedDivisionTemplate lock byte; old native_design_catalog.rs:507 preserves its nonzero test.
pub const LOCKED: usize = 0x235;
/// CReferencedDivisionTemplate obsolete flag byte, bit0; old native_design_catalog.rs:507.
pub const OBSOLETE_FLAGS: usize = 0x236;

use crate::{
    Memory, ccountry,
    cpdx_array::Array,
    csub_unit_definition as subunit,
    memory::{RecordField, RecordLayout, RecordSources},
    msvc_string::{self, Descriptor},
    read, read_slice,
    walk::Outcome,
};

/// Contained composition +190, consumed by 140B90370 for the counter sprite.
pub const PRIORITY_UNIT: usize = COMPOSITION + 0x190;
/// Contained composition +198, preferred by category getter 140B8F7F0.
pub const CATEGORY_UNIT: usize = COMPOSITION + 0x198;
/// Contained composition +1A0, the 3D-model representative RecalculateDerivedStats
/// (140B94FA0) stores; 140B91150 returns it before falling back to 140B90370.
pub const VISUAL_REPRESENTATIVE: usize = COMPOSITION + 0x1a0;
/// Contained composition +A8/+B4: the (CSubUnitDefinition*, i32 count) entries
/// 140B94FA0 groups from the three grids, which 141918950 and 141919100 score.
pub const SUBUNIT_COUNTS: usize = COMPOSITION + 0xa8;
/// A grouped entry: definition pointer, then its signed count.
pub const SUBUNIT_COUNT_STRIDE: usize = 0x10;
/// Contained composition +1A8, saved by 140B977F0 token34ED and consumed by 1418C4B50.
pub const CUSTOM_ICON: usize = COMPOSITION + 0x1a8;
/// Contained composition +1AC, token3525; controls custom-icon use under NATO settings.
pub const CUSTOM_ICON_NATO: usize = COMPOSITION + 0x1ac;
/// Contained composition +218, first nonzero override in category getter 140B8F7F0.
pub const UNIT_TYPE_OVERRIDE: usize = COMPOSITION + 0x218;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

/// One grouped template entry. `token` is zero for a null-object definition.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SubunitCount {
    pub token: u32,
    /// A country-local clone (`CDeploymentStatus+0x20`), not the global definition.
    pub local: bool,
    pub count: i32,
}

/// The daily dictionary row: what the models do not need within the hour.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub template: u64,
    pub owner: u32,
    pub name: Span,
    /// Engine category, NOT a verified frame in the legacy twelve-frame strip.
    pub unit_type: i32,
}

/// What a unit counter and the 3D model draw from a template, which the player
/// can change at any hour: [`walk_icons`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IconRow {
    pub template: u64,
    pub priority_unit: u32,
    pub priority_unit_name: Span,
    pub custom_icon: i32,
    pub custom_icon_nato: u8,
    /// 140B91150's result: +0x1A0, else the priority getter's. Token zero is
    /// the null object, which draws no model. Its count is unused.
    pub representative: SubunitCount,
    /// The stored grouped entries, in native order.
    pub counts: Span,
    /// Composition +0x1D4 (`country`), the clone scope of local definitions and
    /// the model fallback country; raw signed reference.
    pub country: i32,
    /// Composition +0x1DC (`foreign_template_tag`); raw signed reference.
    pub foreign_country: i32,
    /// Saved model category filter, absent when disengaged.
    pub model_category: Option<u64>,
    /// Saved explicit model entity; empty when unset.
    pub model_entity: Span,
    /// Saved excluded equipment variants, full handles in native order.
    pub excluded_variants: Span,
    /// Why this template's model inputs could not be read this hour. The
    /// representative, grouped entries, model entity and excluded variants are
    /// then empty; the counter's fields, the country operands and the model
    /// category, all from the authenticated record, stand.
    pub model_failure: Option<&'static str>,
}

/// [`walk_icons`]'s arena use and demand.
pub struct IconWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
    pub counts_len: usize,
    pub required_counts: usize,
    pub variants_len: usize,
    pub required_variants: usize,
    /// The sub-unit database's length, for both database slot arrays.
    pub required_database: usize,
    /// Slots that would hold any one country's definitions at half load.
    pub required_locals: usize,
}

/// [`walk`]'s arena use and demand.
pub struct Walk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
    /// Definition memo slots that would hold every definition at half load.
    pub required_memo: usize,
}

fn pointer(memory: &impl Memory, at: usize) -> Result<usize, &'static str> {
    read::<u64>(memory, at)
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("template pointer unreadable")
}
fn definition(memory: &impl Memory, base: usize, at: usize) -> Result<Option<u32>, &'static str> {
    if at == 0 {
        return Ok(None);
    }
    let identity = read::<[u8; 17]>(memory, at).ok_or("template subunit unreadable")?;
    authenticate(base, &identity)
}

/// One memo slot: a definition pointer, zero when empty, and its token.
pub type MemoSlot = (usize, u32);

/// Definition pointer to identity for one walk. Templates share a few hundred
/// definitions, so each is authenticated once. Open addressing over the
/// caller's slots, filled to half; past that a definition is read again and
/// its demand counted, so the caller grows the slots off-thread. Both walks
/// share the caller's slots; each starts empty.
struct Memo<'a> {
    slots: &'a mut [MemoSlot],
    used: usize,
    misses: usize,
}

impl<'a> Memo<'a> {
    fn new(slots: &'a mut [MemoSlot]) -> Self {
        slots.fill((0, 0));
        Self {
            slots,
            used: 0,
            misses: 0,
        }
    }

    /// The authenticated token, zero for the null object.
    fn token(&mut self, memory: &impl Memory, base: usize, at: usize) -> Result<u32, &'static str> {
        if at == 0 {
            return Ok(0);
        }
        let mut free = None;
        if !self.slots.is_empty() {
            let len = self.slots.len();
            let hash = (at as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
            let mut slot = (hash >> 32) as usize % len;
            // At most half the slots are used, so an empty one ends the probe.
            loop {
                match self.slots[slot].0 {
                    0 => {
                        free = Some(slot);
                        break;
                    }
                    known if known == at => return Ok(self.slots[slot].1),
                    _ => slot = (slot + 1) % len,
                }
            }
        }
        let found = definition(memory, base, at)?.unwrap_or(0);
        self.misses += 1;
        if let Some(slot) = free
            && (self.used + 1) * 2 <= self.slots.len()
        {
            self.slots[slot] = (at, found);
            self.used += 1;
        }
        Ok(found)
    }

    /// Slots that hold every definition read at half load. A full memo
    /// counts a repeated definition again, which only overestimates.
    fn required(&self) -> usize {
        self.misses.saturating_mul(2)
    }
}

/// A definition's identity bytes (vtable, token, real flag) authenticated: its
/// token, or `None` for the null object.
fn authenticate(base: usize, identity: &[u8]) -> Result<Option<u32>, &'static str> {
    let real = identity[subunit::IS_REAL];
    let expected = if real == 0 {
        subunit::NULL_VTABLE_RVA
    } else {
        subunit::VTABLE_RVA
    };
    if base.checked_add(expected).map(|p| p as u64)
        != Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
    {
        return Err("template subunit identity mismatch");
    }
    if real == 0 {
        return Ok(None);
    }
    let token = u32::from_le_bytes(
        identity[subunit::TOKEN..subunit::TOKEN + 4]
            .try_into()
            .unwrap(),
    );
    // Zero is the replay's null-object marker. A real zero-token definition
    // is unsupported, not silently converted into a null one.
    if token == 0 {
        return Err("real template subunit has unsupported zero token");
    }
    Ok(Some(token))
}

/// Resolves a definition pointer to the token a counter draws.
trait Tokens {
    fn token(&mut self, memory: &impl Memory, base: usize, at: usize) -> Result<u32, &'static str>;
}

impl Tokens for Memo<'_> {
    fn token(&mut self, memory: &impl Memory, base: usize, at: usize) -> Result<u32, &'static str> {
        Memo::token(self, memory, base, at)
    }
}

/// What one gathered read established about a definition. Each part keeps its
/// own failure, so a counter still has its token when the clone flag is
/// unknown, and a failed read is never repeated within a walk.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    token: u32,
    token_failure: Option<&'static str>,
    local: bool,
    local_failure: Option<&'static str>,
}

impl Identity {
    fn token(self) -> Result<u32, &'static str> {
        match self.token_failure {
            Some(reason) => Err(reason),
            None => Ok(self.token),
        }
    }

    fn full(self) -> Result<SubunitCount, &'static str> {
        let token = self.token()?;
        if let Some(reason) = self.local_failure {
            return Err(reason);
        }
        Ok(SubunitCount {
            token,
            local: self.local,
            count: 0,
        })
    }
}

/// A sub-unit database entry, sorted by pointer for [`walk_icons`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GlobalSlot {
    pointer: usize,
    identity: Identity,
    known: bool,
}

/// A definition seen by [`walk_icons`], stamped with the country whose
/// templates named it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LocalSlot {
    pointer: usize,
    stamp: u32,
    identity: Identity,
}

/// Caller-owned storage for [`walk_icons`]'s definition lookups; each grows
/// off-thread from the demand the walk reports.
pub struct DefinitionSlots<'a> {
    /// The sub-unit database's pointer vector, copied once per walk.
    pub database: &'a mut [usize],
    /// The same pointers sorted, with each one's identity once read.
    pub globals: &'a mut [GlobalSlot],
    /// Open-addressed per-country definition identities.
    pub locals: &'a mut [LocalSlot],
}

const DEFINITION_FIELDS: [RecordField; 2] = [
    record_field(0, 0, 17),
    record_field(subunit::DATABASE_INDEX, 20, 4),
];
const DEFINITION_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DEFINITION_FIELDS, 24).expect("valid definition layout");

/// How a walk tells a clone from its global definition (`db[index] != pointer`).
enum Database<'a> {
    /// The database copied once, and its pointers sorted.
    Copied {
        database: &'a [usize],
        globals: &'a mut [GlobalSlot],
    },
    /// Slots too small this hour, or the copy faulted: each lookup reads
    /// `db[index]`. The slots still grow, so a later hour copies.
    Live(subunit::Database),
    /// The database is unreadable: no clone flag, so no model inputs, but
    /// tokens still resolve.
    Unavailable(&'static str),
}

/// Definition identities for one hourly walk, laid out for the cache. The
/// database is copied once, so whether a definition is a clone needs no
/// further read. A database entry is found by binary search over a few hundred
/// sorted pointers and read once a walk. Every definition a country's templates
/// name is kept in a small table that starts over for each country, because
/// clones belong to one country's templates. A definition's identity and index
/// come in one gathered read, and its outcome, failures included, is kept, so
/// no pointer is read twice while the slots have room. A database that does not
/// fit its slots, or cannot be read, degrades the walk rather than failing it.
struct Definitions<'a> {
    database: Database<'a>,
    locals: &'a mut [LocalSlot],
    stamp: u32,
    /// The table's power-of-two length less one; zero turns caching off.
    mask: usize,
    used: usize,
    misses: usize,
    required_locals: usize,
}

/// Copies the database's pointer vector into `database`.
fn copy_database(
    memory: &impl Memory,
    vector: Array,
    database: &mut [usize],
) -> Result<(), &'static str> {
    let mut chunk = [0_u8; BATCH * 8];
    for first in (0..database.len()).step_by(BATCH) {
        let len = (database.len() - first).min(BATCH);
        let at = vector
            .element(first as u32, 8)
            .ok_or("subunit database index invalid")?;
        if !read_slice(memory, at, &mut chunk[..len * 8]) {
            return Err("subunit database unreadable");
        }
        for (slot, bytes) in database[first..first + len]
            .iter_mut()
            .zip(chunk[..len * 8].chunks_exact(8))
        {
            *slot = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                .map_err(|_| "subunit database pointer invalid")?;
        }
    }
    Ok(())
}

impl<'a> Definitions<'a> {
    /// Copies the live database into the slots when it fits. `required`
    /// receives its length whenever it is readable.
    fn load(
        memory: &impl Memory,
        base: usize,
        slots: DefinitionSlots<'a>,
        required: &mut usize,
    ) -> Self {
        let database = match subunit::Database::read(memory, base) {
            Err(reason) => Database::Unavailable(reason),
            Ok(live) => {
                let vector = live.vector();
                let count = vector.count as usize;
                *required = count;
                if count > slots.database.len() || count > slots.globals.len() {
                    Database::Live(live)
                } else {
                    let database = &mut slots.database[..count];
                    match copy_database(memory, vector, database) {
                        // The header was read; each lookup reads its entry.
                        Err(_) => Database::Live(live),
                        Ok(()) => {
                            let globals = &mut slots.globals[..count];
                            for (slot, &pointer) in globals.iter_mut().zip(database.iter()) {
                                *slot = GlobalSlot {
                                    pointer,
                                    ..GlobalSlot::default()
                                };
                            }
                            globals.sort_unstable_by_key(|slot| slot.pointer);
                            Database::Copied { database, globals }
                        }
                    }
                }
            }
        };
        // The largest power of two the slots hold, so a probe masks.
        let usable = match slots.locals.len() {
            0 | 1 => 0,
            len => 1 << (usize::BITS - 1 - len.leading_zeros()),
        };
        let locals = &mut slots.locals[..usable];
        locals.fill(LocalSlot::default());
        Self {
            database,
            mask: usable.saturating_sub(1),
            locals,
            stamp: 0,
            used: 0,
            misses: 0,
            required_locals: 0,
        }
    }

    /// Starts a country's templates: earlier countries' clones no longer match.
    fn begin_country(&mut self) {
        self.required_locals = self.required_locals.max(self.misses.saturating_mul(2));
        self.stamp = self.stamp.wrapping_add(1).max(1);
        self.used = 0;
        self.misses = 0;
    }

    /// A power of two of slots that would hold any one country's
    /// definitions at half load.
    fn required_locals(&self) -> usize {
        self.required_locals
            .max(self.misses.saturating_mul(2))
            .checked_next_power_of_two()
            .unwrap_or(usize::MAX)
    }

    /// `db[index]`, the global definition an index names, as the engine's own
    /// lookup validates it.
    fn entry(&self, memory: &impl Memory, index: i32) -> Result<usize, &'static str> {
        match &self.database {
            Database::Copied { database, .. } => u32::try_from(index)
                .ok()
                .filter(|index| *index != 0)
                .and_then(|index| database.get(index as usize))
                .copied()
                .filter(|global| *global != 0)
                .ok_or("subunit database index outside the database"),
            Database::Live(live) => live.entry(memory, index),
            Database::Unavailable(reason) => Err(reason),
        }
    }

    /// One gathered read of a definition's identity and database index. The
    /// token does not depend on the index being readable.
    fn fetch(&self, memory: &impl Memory, base: usize, at: usize) -> Identity {
        let mut record = [0_u8; 24];
        let completed = memory.copy_records(
            RecordSources::Pointers(&[at]),
            &DEFINITION_LAYOUT,
            &mut record,
        );
        if completed == 0 {
            return Identity {
                token_failure: Some("template subunit unreadable"),
                ..Identity::default()
            };
        }
        let token = match authenticate(base, &record[..17]) {
            Err(reason) => {
                return Identity {
                    token_failure: Some(reason),
                    ..Identity::default()
                };
            }
            // The null object has no clone flag to read.
            Ok(None) => return Identity::default(),
            Ok(Some(token)) => token,
        };
        let local = if completed < DEFINITION_FIELDS.len() {
            Err("subunit database index unreadable")
        } else {
            self.entry(memory, dword(&record, 20))
                .map(|global| global != at)
        };
        Identity {
            token,
            token_failure: None,
            local: local.unwrap_or(false),
            local_failure: local.err(),
        }
    }

    /// Keeps a definition for the rest of the country's templates when the
    /// table has room; demand is counted either way.
    fn remember(&mut self, free: Option<usize>, at: usize, identity: Identity) {
        self.misses += 1;
        if let Some(slot) = free
            && (self.used + 1) * 2 <= self.mask + 1
        {
            self.locals[slot] = LocalSlot {
                pointer: at,
                stamp: self.stamp,
                identity,
            };
            self.used += 1;
        }
    }

    /// A definition's identity: this country's table, then the sorted
    /// database, then one read.
    fn lookup(&mut self, memory: &impl Memory, base: usize, at: usize) -> Identity {
        if at == 0 {
            return Identity::default();
        }
        let mut free = None;
        if self.mask != 0 {
            let hash = (at as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
            let mut slot = (hash >> 32) as usize & self.mask;
            // At most half the slots hold this country's definitions, so a
            // slot of an earlier country or an empty one ends the probe.
            loop {
                let entry = self.locals[slot];
                if entry.stamp != self.stamp {
                    free = Some(slot);
                    break;
                }
                if entry.pointer == at {
                    return entry.identity;
                }
                slot = (slot + 1) & self.mask;
            }
        }
        let found = match &self.database {
            Database::Copied { globals, .. } => {
                globals.binary_search_by_key(&at, |slot| slot.pointer).ok()
            }
            _ => None,
        };
        let known = match (&self.database, found) {
            (Database::Copied { globals, .. }, Some(found)) if globals[found].known => {
                Some(globals[found].identity)
            }
            _ => None,
        };
        let identity = match known {
            Some(identity) => identity,
            None => {
                let identity = self.fetch(memory, base, at);
                if let (Database::Copied { globals, .. }, Some(found)) = (&mut self.database, found)
                {
                    globals[found].identity = identity;
                    globals[found].known = true;
                }
                identity
            }
        };
        self.remember(free, at, identity);
        identity
    }

    /// The token and clone flag, or why either is unknown.
    fn identify(
        &mut self,
        memory: &impl Memory,
        base: usize,
        at: usize,
    ) -> Result<SubunitCount, &'static str> {
        self.lookup(memory, base, at).full()
    }
}

impl Tokens for Definitions<'_> {
    fn token(&mut self, memory: &impl Memory, base: usize, at: usize) -> Result<u32, &'static str> {
        self.lookup(memory, base, at).token()
    }
}

/// 140B90370's fallback when +0x190 is null: regiment cell (0,0) when
/// 140FAA3F0's low-16-bit real-unit count is nonzero, else the null singleton.
/// The native wrap is kept.
fn fallback_priority_unit(
    memory: &impl Memory,
    base: usize,
    memo: &mut impl Tokens,
    at: usize,
) -> Result<usize, &'static str> {
    let regiments = at
        .checked_add(REGIMENT_DATA)
        .and_then(|header| Array::read(memory, header, 8))
        .ok_or("template grid invalid")?;
    let mut real_count = 0_u16;
    for cell in 0..regiments.count {
        let p = pointer(
            memory,
            regiments
                .element(cell, 8)
                .ok_or("template grid cell invalid")?,
        )?;
        real_count = real_count.wrapping_add(u16::from(memo.token(memory, base, p)? != 0));
    }
    let selected = if real_count != 0 {
        pointer(
            memory,
            regiments
                .element(0, 8)
                .ok_or("template first cell missing")?,
        )?
    } else {
        pointer(
            memory,
            base.checked_add(subunit::NULL_OBJECT_RVA)
                .ok_or("template null singleton overflow")?,
        )?
    };
    if selected == 0 {
        return Err("template selected subunit is null");
    }
    Ok(selected)
}

const fn record_field(source_offset: usize, destination_offset: usize, len: usize) -> RecordField {
    RecordField {
        source_offset,
        destination_offset,
        len,
    }
}

/// Composition +0x190 (priority unit) through the unit-type override: one span
/// holding every fixed scalar and the model entity string's header.
const TAIL: usize = PRIORITY_UNIT;
const TAIL_END: usize = UNIT_TYPE_OVERRIDE + 4;
/// Where the tail sits in both records, after three 16-byte-aligned spans.
const TAIL_AT: usize = 48;
const RECORD: usize = TAIL_AT + TAIL_END - TAIL;
/// The daily row's fixed fields: identity, name header and the tail.
const DAILY_FIELDS: [RecordField; 3] = [
    record_field(0, 0, 16),
    record_field(NAME, 16, msvc_string::STRIDE),
    record_field(TAIL, TAIL_AT, TAIL_END - TAIL),
];
const DAILY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DAILY_FIELDS, RECORD).expect("valid template layout");
/// The hourly row's fixed fields: identity, then one contiguous span from the
/// grouped entries' header through the model entity string, which holds every
/// header, the representative, the icon fields, the country operands and the
/// model category.
const ICON_SPAN: usize = SUBUNIT_COUNTS;
const ICON_SPAN_END: usize = MODEL_ENTITY + msvc_string::STRIDE;
const ICON_RECORD: usize = 16 + ICON_SPAN_END - ICON_SPAN;
const ICON_FIELDS: [RecordField; 2] = [
    record_field(0, 0, 16),
    record_field(ICON_SPAN, 16, ICON_SPAN_END - ICON_SPAN),
];
const ICON_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ICON_FIELDS, ICON_RECORD).expect("valid template icon layout");
/// The larger of the two records, for the batch buffer.
const MAX_RECORD: usize = if RECORD > ICON_RECORD {
    RECORD
} else {
    ICON_RECORD
};

/// An icon-span field's place in the icon record.
const fn icon(offset: usize) -> usize {
    16 + offset - ICON_SPAN
}
const BATCH: usize = 32;

/// A tail field's place in a record.
const fn tail(offset: usize) -> usize {
    TAIL_AT + offset - TAIL
}
fn word(record: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(record[at..at + 8].try_into().unwrap())
}
fn dword(record: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(record[at..at + 4].try_into().unwrap())
}

/// Copies a span of pointer-sized entries, `BATCH` at a time.
fn pointers(
    memory: &impl Memory,
    array: Array,
    first: u32,
    output: &mut [usize],
) -> Result<(), &'static str> {
    let source = array
        .slice(first, output.len() as u32, 8)
        .map_err(|_| "template pointer index invalid")?;
    // Validate in element order: an earlier null wins over a later unreadable pointer.
    crate::cpdx_array::pointers(
        memory,
        source,
        "template pointers unreadable",
        |index, pointer| {
            if pointer == 0 {
                return Err("template pointer null");
            }
            output[index as usize] = pointer;
            Ok(())
        },
    )
}

/// Every country's templates in roster order. Each roster batch's fixed fields
/// are hinted and then gathered with `layout`; `prepare` sees the batch's
/// records first, to hint what the visits will read, and `visit` receives the
/// owner's id, the template and its authenticated record. Hints read nothing,
/// so a batch's cache misses overlap instead of following one another.
fn visit_templates(
    memory: &impl Memory,
    base: usize,
    state: usize,
    layout: &RecordLayout<'_>,
    mut prepare: impl FnMut(&[u8], usize),
    mut visit: impl FnMut(u32, usize, &[u8]) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let vtable = base
        .checked_add(VTABLE_RVA)
        .ok_or("division template module address overflow")? as u64;
    let mut found_country = false;
    let mut templates = [0_usize; BATCH];
    let stride = layout.stride();
    let mut records = [0_u8; BATCH * MAX_RECORD];
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        found_country = true;
        let root = country
            .pointer
            .checked_add(ccountry::DIVISION_TEMPLATES)
            .ok_or("template roster overflow")?;
        let roster = Array::read(memory, root, 8).ok_or("template roster invalid")?;
        for first in (0..roster.count).step_by(BATCH) {
            let batch = ((roster.count - first) as usize).min(BATCH);
            pointers(memory, roster, first, &mut templates[..batch])?;
            for &at in &templates[..batch] {
                for field in layout.fields() {
                    memory.prefetch(at.wrapping_add(field.source_offset), field.len);
                }
            }
            if memory.copy_records(
                RecordSources::Pointers(&templates[..batch]),
                layout,
                &mut records[..batch * stride],
            ) < batch * layout.field_count()
            {
                return Err("template fields unreadable");
            }
            prepare(&records[..batch * stride], stride);
            for (&at, record) in templates[..batch]
                .iter()
                .zip(records[..batch * stride].chunks_exact(stride))
            {
                if word(record, 0) != vtable || word(record, 8) as u32 != REFERENCE_TYPE {
                    return Err("division template vtable or reference type mismatch");
                }
                visit(country.id, at, record)?;
            }
        }
    }
    if !found_country {
        return Err("template census contains no countries");
    }
    Ok(())
}

/// 140B90370's sub-unit: the stored priority unit, else its fallback.
fn selected_unit(
    memory: &impl Memory,
    base: usize,
    memo: &mut impl Tokens,
    at: usize,
    stored: u64,
) -> Result<usize, &'static str> {
    match stored as usize {
        0 => fallback_priority_unit(memory, base, memo, at),
        selected => Ok(selected),
    }
}

fn outcome(len: usize, missing: u32) -> Outcome {
    if missing == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated {
            len,
            unread_rows: missing,
        }
    }
}

/// Complete country template rosters, including unused templates: exact names
/// and the unit_type category, which the models do not need within the hour.
/// Capacity omits a WHOLE template and grows the caller's arena off-thread. No
/// engine getters are invoked. Each roster batch is gathered in three spans a
/// template; the sub-unit behind a zero category is authenticated once a walk.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    state: usize,
    rows: &mut [Row],
    bytes: &mut [u8],
    memo: &mut [MemoSlot],
) -> Walk {
    let mut result = Walk {
        outcome: Outcome::Rows { len: 0 },
        bytes_len: 0,
        required_bytes: 0,
        required_memo: 0,
    };
    let mut memo = Memo::new(memo);
    let (mut len, mut missing) = (0, 0_u32);
    let captured = visit_templates(
        memory,
        base,
        state,
        &DAILY_LAYOUT,
        |_, _| {},
        |owner, at, record| {
            let name = Descriptor::from_header(
                memory,
                at.checked_add(NAME)
                    .ok_or("template name address overflow")?,
                record[16..48].try_into().unwrap(),
            )?;
            let mut unit_type = dword(record, tail(UNIT_TYPE_OVERRIDE));
            if unit_type == 0 {
                let category = match word(record, tail(CATEGORY_UNIT)) as usize {
                    0 => selected_unit(
                        memory,
                        base,
                        &mut memo,
                        at,
                        word(record, tail(PRIORITY_UNIT)),
                    )?,
                    category => category,
                };
                memo.token(memory, base, category)?;
                unit_type = subunit::template_category(memory, category)?;
            }
            result.required_bytes = result
                .required_bytes
                .checked_add(name.len)
                .ok_or("template byte arena overflow")?;
            if len == rows.len() || name.len > bytes.len() - result.bytes_len {
                missing = missing
                    .checked_add(1)
                    .ok_or("template row count overflow")?;
                return Ok(());
            }
            let start = result.bytes_len;
            name.copy(memory, &mut bytes[start..])?;
            result.bytes_len += name.len;
            rows[len] = Row {
                template: word(record, 8),
                owner,
                name: Span {
                    start,
                    len: name.len,
                },
                unit_type,
            };
            len += 1;
            Ok(())
        },
    );
    result.required_memo = memo.required();
    result.outcome = match captured {
        Ok(()) => outcome(len, missing),
        Err(reason) => Outcome::Unavailable { reason },
    };
    result
}

/// Every country template's hourly inputs ([`IconRow`]), roster by roster in
/// the order [`walk`] visits them: what the counter draws and the stored
/// results the 3D model selectors read (the representative, the grouped
/// (definition, count) entries and the saved model choices). A player's edit
/// reaches both within the hour. Capacity omits a WHOLE row, as in [`walk`].
/// A fault in a template's model inputs (its grouped entries, exclusions, model
/// string or a definition's clone flag) fails only that row's model fields, so
/// every counter keeps its icon; a sub-unit database too large for its slots
/// is read live and the slots grow.
/// Each roster batch is gathered in two spans a template (identity and one
/// contiguous span through the model entity), the grouped entries in one more,
/// and definitions resolve through `Definitions`.
#[allow(clippy::too_many_arguments)]
pub fn walk_icons(
    memory: &impl Memory,
    base: usize,
    state: usize,
    rows: &mut [IconRow],
    bytes: &mut [u8],
    counts: &mut [SubunitCount],
    variants: &mut [u64],
    slots: DefinitionSlots<'_>,
) -> IconWalk {
    let mut result = IconWalk {
        outcome: Outcome::Rows { len: 0 },
        bytes_len: 0,
        required_bytes: 0,
        counts_len: 0,
        required_counts: 0,
        variants_len: 0,
        required_variants: 0,
        required_database: 0,
        required_locals: 0,
    };
    let (mut len, mut missing) = (0, 0_u32);
    let mut owner = None;
    let mut grouped_scratch = [0_u8; BATCH * SUBUNIT_COUNT_STRIDE];
    let mut excluded_scratch = [0_usize; BATCH];
    let mut memo = Definitions::load(memory, base, slots, &mut result.required_database);
    let hint = |records: &[u8], stride: usize| {
        for record in records.chunks_exact(stride) {
            for (header, stride) in [
                (icon(SUBUNIT_COUNTS), SUBUNIT_COUNT_STRIDE),
                (icon(EXCLUDED_VARIANT_DATA), 8),
            ] {
                let count =
                    u32::from_le_bytes(record[header + 12..header + 16].try_into().unwrap());
                let data = word(record, header) as usize;
                memory.prefetch(data, (count as usize).saturating_mul(stride));
            }
        }
    };
    let captured = visit_templates(
        memory,
        base,
        state,
        &ICON_LAYOUT,
        hint,
        |country, at, record| {
            if owner != Some(country) {
                owner = Some(country);
                memo.begin_country();
            }
            // What the counter draws, as L3a reads it: a failure here fails
            // the hour's table.
            let selected = selected_unit(
                memory,
                base,
                &mut memo,
                at,
                word(record, icon(PRIORITY_UNIT)),
            )?;
            let priority_unit = memo.token(memory, base, selected)?;
            let selected_name = if priority_unit != 0 {
                Some(Descriptor::read(
                    memory,
                    selected
                        .checked_add(subunit::NAME)
                        .ok_or("subunit name overflow")?,
                )?)
            } else {
                None
            };
            // The model inputs' headers, read with the record. Their demand is
            // counted before any capacity check, so every arena grows at once.
            let headers = (|| -> Result<(Array, Array, Descriptor), &'static str> {
                let header = |at: usize| -> [u8; 16] { record[at..at + 16].try_into().unwrap() };
                let grouped =
                    Array::from_header(header(icon(SUBUNIT_COUNTS)), SUBUNIT_COUNT_STRIDE)
                        .ok_or("template grouped subunits invalid")?;
                let exclusions = Array::from_header(header(icon(EXCLUDED_VARIANT_DATA)), 8)
                    .ok_or("template variant exclusions invalid")?;
                let entity = icon(MODEL_ENTITY);
                let model_entity = Descriptor::from_header(
                    memory,
                    at.checked_add(MODEL_ENTITY)
                        .ok_or("template model entity address overflow")?,
                    record[entity..entity + msvc_string::STRIDE]
                        .try_into()
                        .unwrap(),
                )?;
                result.required_bytes = result
                    .required_bytes
                    .checked_add(model_entity.len)
                    .ok_or("template byte arena overflow")?;
                result.required_counts = result
                    .required_counts
                    .checked_add(grouped.count as usize)
                    .ok_or("template grouped arena overflow")?;
                result.required_variants = result
                    .required_variants
                    .checked_add(exclusions.count as usize)
                    .ok_or("template variant arena overflow")?;
                Ok((grouped, exclusions, model_entity))
            })();
            let name_len = selected_name.as_ref().map_or(0, |name| name.len);
            result.required_bytes = result
                .required_bytes
                .checked_add(name_len)
                .ok_or("template byte arena overflow")?;
            if len == rows.len() || name_len > bytes.len() - result.bytes_len {
                missing = missing
                    .checked_add(1)
                    .ok_or("template row count overflow")?;
                return Ok(());
            }
            let row_start = (result.bytes_len, result.counts_len, result.variants_len);
            let mut row = IconRow {
                template: word(record, 8),
                priority_unit,
                priority_unit_name: Span {
                    start: result.bytes_len,
                    len: name_len,
                },
                custom_icon: dword(record, icon(CUSTOM_ICON)),
                custom_icon_nato: record[icon(CUSTOM_ICON_NATO)],
                // Read with the authenticated record, so they cannot fail.
                country: dword(record, icon(COUNTRY_OPERAND_289A)),
                foreign_country: dword(record, icon(COUNTRY_OPERAND_3ABE)),
                // The optional's inactive payload is not an input.
                model_category: (record[icon(MODEL_CATEGORY_PRESENT)] != 0)
                    .then(|| word(record, icon(MODEL_CATEGORY))),
                ..IconRow::default()
            };
            if let Some(name) = &selected_name {
                name.copy(memory, &mut bytes[result.bytes_len..])?;
                result.bytes_len += name.len;
            }
            // The 3D model's inputs. A fault here fails only this row's model
            // fields; the counter keeps its icon. `Ok(false)` is a row that does
            // not fit, omitted whole.
            let model = (|| -> Result<bool, &'static str> {
                let (grouped, exclusions, model_entity) = headers?;
                if model_entity.len > bytes.len() - result.bytes_len
                    || grouped.count as usize > counts.len() - result.counts_len
                    || exclusions.count as usize > variants.len() - result.variants_len
                {
                    return Ok(false);
                }
                // 140B91150: +0x1A0, else the priority getter's result.
                row.representative = match word(record, icon(VISUAL_REPRESENTATIVE)) as usize {
                    0 => memo.identify(memory, base, selected)?,
                    representative => memo.identify(memory, base, representative)?,
                };
                row.model_entity = Span {
                    start: result.bytes_len,
                    len: model_entity.len,
                };
                model_entity.copy(memory, &mut bytes[result.bytes_len..])?;
                result.bytes_len += model_entity.len;
                row.counts = Span {
                    start: result.counts_len,
                    len: grouped.count as usize,
                };
                let entries = &mut grouped_scratch;
                for first in (0..grouped.count).step_by(BATCH) {
                    let span = ((grouped.count - first) as usize).min(BATCH) * SUBUNIT_COUNT_STRIDE;
                    let at = grouped
                        .element(first, SUBUNIT_COUNT_STRIDE)
                        .ok_or("template grouped subunit index invalid")?;
                    if !read_slice(memory, at, &mut entries[..span]) {
                        return Err("template grouped subunits unreadable");
                    }
                    for entry in entries[..span].chunks_exact(SUBUNIT_COUNT_STRIDE) {
                        let definition =
                            usize::try_from(u64::from_le_bytes(entry[..8].try_into().unwrap()))
                                .map_err(|_| "template grouped subunit pointer invalid")?;
                        counts[result.counts_len] = SubunitCount {
                            count: i32::from_le_bytes(entry[8..12].try_into().unwrap()),
                            ..memo.identify(memory, base, definition)?
                        };
                        result.counts_len += 1;
                    }
                }
                row.excluded_variants = Span {
                    start: result.variants_len,
                    len: exclusions.count as usize,
                };
                let excluded = &mut excluded_scratch;
                for first in (0..exclusions.count).step_by(BATCH) {
                    let batch = ((exclusions.count - first) as usize).min(BATCH);
                    pointers(memory, exclusions, first, &mut excluded[..batch])?;
                    for &variant in &excluded[..batch] {
                        variants[result.variants_len] =
                            crate::cequipment_variant::identity(memory, base, variant)?;
                        result.variants_len += 1;
                    }
                }
                Ok(true)
            })();
            match model {
                Ok(true) => {}
                Ok(false) => {
                    // The whole row waits for room, name included.
                    (result.bytes_len, result.counts_len, result.variants_len) = row_start;
                    missing = missing
                        .checked_add(1)
                        .ok_or("template row count overflow")?;
                    return Ok(());
                }
                Err(reason) => {
                    // Keep the counter's fields and the name; drop the rest.
                    let name_end = row_start.0 + name_len;
                    (result.bytes_len, result.counts_len, result.variants_len) =
                        (name_end, row_start.1, row_start.2);
                    row = IconRow {
                        template: row.template,
                        priority_unit: row.priority_unit,
                        priority_unit_name: row.priority_unit_name,
                        custom_icon: row.custom_icon,
                        custom_icon_nato: row.custom_icon_nato,
                        country: row.country,
                        foreign_country: row.foreign_country,
                        model_category: row.model_category,
                        model_entity: Span {
                            start: name_end,
                            len: 0,
                        },
                        counts: Span {
                            start: row_start.1,
                            len: 0,
                        },
                        excluded_variants: Span {
                            start: row_start.2,
                            len: 0,
                        },
                        model_failure: Some(reason),
                        ..IconRow::default()
                    };
                }
            }
            rows[len] = row;
            len += 1;
            Ok(())
        },
    );
    result.required_locals = memo.required_locals();
    result.outcome = match captured {
        Ok(()) => outcome(len, missing),
        Err(reason) => Outcome::Unavailable { reason },
    };
    result
}

/// Authenticates a template pointer and returns its full native handle: the
/// reference type in the low word and the id in the high word. Callers that
/// accept a null pointer must test for it before calling.
pub(crate) fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<u64, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("division template module address overflow")?;
    let bytes = read::<[u8; 16]>(memory, pointer).ok_or("division template identity unreadable")?;
    let handle = u64::from_le_bytes(bytes[8..].try_into().unwrap());
    if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != expected as u64
        || handle as u32 != REFERENCE_TYPE
    {
        return Err("division template vtable or reference type mismatch");
    }
    Ok(handle)
}

#[cfg(test)]
mod tests;
