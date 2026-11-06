//! Referenced division-template identity and retained model preferences.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x029c_c350;
/// CReferencedDivisionTemplate reference type.
pub const REFERENCE_TYPE: u32 = 52;
/// CReferencedDivisionTemplate u32 type word.
pub const TYPE: usize = 0x08;
/// Division fog-reveal skip byte read from the selected template.
pub const REVEAL_SKIP: usize = 0x234;
/// CReferencedDivisionTemplate u32 id.
pub const ID: usize = 0x0c;
/// Contained composition storage, separate from a base subobject.
pub const COMPOSITION: usize = 0x18;
/// CReferencedDivisionTemplate name MSVC string.
pub const NAME: usize = 0x20;
/// Composition fallback localisation key.
pub const NAME_KEY: usize = COMPOSITION + 0x28;
/// CReferencedDivisionTemplate grid dimensions, columns low16/rows high16.
pub const GRID_DIMENSIONS: usize = 0x60;
/// CReferencedDivisionTemplate regiment CSubUnitDefinition pointer-vector data.
pub const REGIMENT_DATA: usize = 0x68;
/// CReferencedDivisionTemplate support CSubUnitDefinition pointer-vector data.
pub const SUPPORT_DATA: usize = 0x88;
/// CReferencedDivisionTemplate regimental-support definition pointer-vector data.
pub const REGIMENTAL_SUPPORT_DATA: usize = 0xa8;
/// The three composition grids' pointer-vector headers, in the order
/// [`IconRow::grids`] keeps them: regiments, support, regimental support. A
/// cell holds a definition pointer; a null pointer is an empty cell.
pub const GRIDS: [usize; 3] = [REGIMENT_DATA, SUPPORT_DATA, REGIMENTAL_SUPPORT_DATA];
/// Excluded equipment-variant pointer-array data.
pub const EXCLUDED_VARIANT_DATA: usize = 0x140;
/// CReferencedDivisionTemplate u32 priority.
pub const PRIORITY: usize = 0x1a4;
/// Raw headquarters-template byte retained without boolean collapse.
pub const IS_ARMY_HQ: usize = 0x1cc;
/// CReferencedDivisionTemplate token289A country operand i32.
pub const COUNTRY_OPERAND_289A: usize = 0x1ec;
/// CReferencedDivisionTemplate token3ABE country operand i32.
pub const COUNTRY_OPERAND_3ABE: usize = 0x1f4;
/// Raw model category, u64, meaningful only when its presence byte is set.
pub const MODEL_CATEGORY: usize = 0x200;
/// Model-category presence byte.
pub const MODEL_CATEGORY_PRESENT: usize = 0x208;
/// Model entity-name string; retain exact engine-owned bytes.
pub const MODEL_ENTITY: usize = 0x210;
/// Lock byte tested for nonzero.
pub const LOCKED: usize = 0x235;
/// Retained obsolete byte; serialization tests it for nonzero.
pub const OBSOLETE_FLAGS: usize = 0x236;
/// Template parent reference, kept independently of roster membership.
pub const PARENT: usize = COMPOSITION + 0x170;
/// Equipment eligibility switches, kept as native bytes.
pub const ALLOW_NEW_EQUIPMENT: usize = COMPOSITION + 0x1ad;
pub const ALLOW_FOREIGN_EQUIPMENT: usize = COMPOSITION + 0x1ae;
/// Equipment types the template forbids, as definition pointers.
pub const FORBIDDEN_EQUIPMENT: usize = COMPOSITION + 0x140;
/// Original creator country, independent of the current composition owner.
pub const CREATOR_COUNTRY: usize = COMPOSITION + 0x1d8;
pub const ORIGIN_TYPE: usize = COMPOSITION + 0x1e0;
pub const FAKE_INTEL: usize = COMPOSITION + 0x21c;
pub const OBSOLETE_CHANGE_DATE: usize = COMPOSITION + 0x228;
pub const FORCE_ALLOW_RECRUITING: usize = COMPOSITION + 0x238;
pub const DIVISION_CAP: usize = COMPOSITION + 0x23c;
pub const DIVISION_CAP_PRESENT: usize = COMPOSITION + 0x240;
pub const EQUIPMENT_NICHE: usize = COMPOSITION + 0x244;
/// Explicit template role and the AI's stored role selection.
pub const ROLE: usize = COMPOSITION + 0x1c0;
pub const ROLE_PRESENT: usize = COMPOSITION + 0x1c4;
pub const AI_ROLE: usize = COMPOSITION + 0x1c8;
pub const AI_ROLE_PRESENT: usize = COMPOSITION + 0x1cc;
pub const AI_ROLE_FALLBACK: usize = COMPOSITION + 0x1d0;
/// Cached composition operands rebuilt with its grouped definitions.
pub const TOTAL_MANPOWER: usize = COMPOSITION + 0x178;
pub const COMMAND_POWER_COST: usize = COMPOSITION + 0x180;
pub const TRAINING_TIME: usize = COMPOSITION + 0x188;

/// Authenticated stored template settings; inactive optional payloads are omitted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub roster_ordinal: u32,
    pub priority: i32,
    pub is_army_hq: u8,
    pub locked: u8,
    pub obsolete: u8,
    pub obsolete_change_date: i32,
    pub force_allow_recruiting: u8,
    pub allow_new_equipment: u8,
    pub allow_foreign_equipment: u8,
    pub parent: u64,
    pub creator_country: i32,
    pub origin_type: i32,
    pub fake_intel: u8,
    pub division_cap: Option<i32>,
    pub equipment_niche: u8,
    pub role: Option<u32>,
    pub ai_role: Option<u32>,
    pub ai_role_fallback: u32,
    pub total_manpower: i32,
    pub command_power_cost: i64,
    pub training_time: i32,
}

use crate::{
    Memory, ccountry,
    cpdx_array::Array,
    csub_unit_definition as subunit,
    memory::{RecordField, RecordLayout, RecordSources},
    msvc_string::{self, Descriptor},
    read, read_slice,
    walk::{Buffer, Outcome},
};

/// Stored composition priority subunit used by the counter sprite.
pub const PRIORITY_UNIT: usize = COMPOSITION + 0x190;
/// Stored composition preferred category subunit.
pub const CATEGORY_UNIT: usize = COMPOSITION + 0x198;
/// Stored representative model subunit selected before priority fallback.
pub const VISUAL_REPRESENTATIVE: usize = COMPOSITION + 0x1a0;
/// Contained composition equipment counts grouped from the three grids
/// and used by model scoring.
pub const SUBUNIT_COUNTS: usize = COMPOSITION + 0xa8;
/// A grouped entry: definition pointer, then its signed count.
pub const SUBUNIT_COUNT_STRIDE: usize = 0x10;
/// Stored composition active-weight operand, serialized and used by counters.
pub const CUSTOM_ICON: usize = COMPOSITION + 0x1a8;
/// Contained composition flag, token3525; controls custom-icon use under NATO settings.
pub const CUSTOM_ICON_NATO: usize = COMPOSITION + 0x1ac;
/// Contained composition signed count added with i32 wrapping for formation
/// division counts. The hourly fixed span already contains this word.
pub const FORMATION_DIVISION_COUNT: usize = COMPOSITION + 0x1b0;
/// Composition origin used by the ordinary manpower template predicate.
/// First nonzero composition category override.
pub const UNIT_TYPE_OVERRIDE: usize = COMPOSITION + 0x218;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

/// One grouped template entry. `token` is zero for a null-object definition.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SubunitCount {
    pub width: Option<i64>,
    pub token: u32,
    /// A country-local clone (in the deployment status's clone vector), not the global definition.
    pub local: bool,
    pub count: i32,
}

/// The dictionary row: a template's handle, roster owner, names and unit_type.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub template: u64,
    pub owner: u32,
    pub name: Span,
    pub name_key: Span,
    /// Engine category, NOT a verified frame in the legacy twelve-frame strip.
    pub unit_type: i32,
}

/// What a unit counter and the 3D model draw from a template, which the player
/// can change at any hour: [`walk_icons`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IconRow {
    pub manpower_exile: bool,
    /// Selected historical naming-group key; None is a null group.
    pub names_group: Option<Span>,
    pub template: u64,
    /// The template object, for the caller's per-template reads.
    pub pointer: usize,
    /// The raw id of the country whose roster lists it.
    pub owner: u32,
    /// The dictionary's unit_type ([`walk`]'s), from the same record;
    /// `unit_type_failure` says why it is unknown.
    pub unit_type: i32,
    pub unit_type_failure: Option<&'static str>,
    pub priority_unit: u32,
    pub priority_unit_name: Span,
    pub custom_icon: i32,
    pub custom_icon_nato: u8,
    /// Raw composition count for the ability's late-join denominator. It is
    /// independent of the model fields and retains its signed native value.
    pub formation_division_count: Option<i32>,
    /// Independently acquired display-name operands in the caller's name arena.
    pub display_name: Option<DisplayName>,
    /// The fallback key in the name arena, read beside the literal for the
    /// dictionary; None when the literal or the key was unreadable.
    pub name_key: Option<Span>,
    /// Stored representative model or priority fallback. Token zero is the null
    /// object and draws no model; its count is unused.
    pub representative: SubunitCount,
    /// The stored grouped entries, in native order.
    pub counts: Span,
    /// Each of [`GRIDS`]' entries in `counts`: the definitions its non-null
    /// cells hold, grouped by token and clone flag in the order each first
    /// occurs, each counted once per cell. A null-object cell is token zero.
    pub grids: [Span; 3],
    /// Every grid cell in native column-major order. A count of zero denotes
    /// an empty pointer; one denotes an authenticated definition, including
    /// the null-object definition. Uses the same counts arena as `grids`.
    pub cells: [Span; 3],
    /// Each grid's columns in the low word and rows in the high word.
    pub grid_dimensions: [u32; 3],
    /// True only when the caller demanded fresh extension operands.
    pub details_fresh: bool,
    /// Failed extension acquisition never withholds the original icon/model fields.
    pub details_failure: Option<&'static str>,
    pub settings: Settings,
    /// Definition tokens in native order, in the variants arena.
    pub forbidden_equipment: Span,
    pub equipment_failure: Option<&'static str>,
    /// The composition's `country`, the clone scope of local definitions and the model fallback
    /// country; raw signed reference.
    pub country: i32,
    /// The composition's `foreign_template_tag`; raw signed reference.
    pub foreign_country: i32,
    /// Saved model category filter, absent when disengaged.
    pub model_category: Option<u64>,
    /// Saved explicit model entity; empty when unset.
    pub model_entity: Span,
    /// Saved excluded equipment variants, full handles in native order.
    pub excluded_variants: Span,
    /// Why this template's model inputs could not be read this hour. The
    /// representative, grouped entries, grids, model entity and excluded
    /// variants are then empty; the counter's fields, the country operands and
    /// the model category, all from the authenticated record, stand.
    pub model_failure: Option<&'static str>,
}

/// [`walk_icons`]'s arena use and demand.
pub struct IconWalk {
    pub outcome: Outcome,
    pub name_bytes_len: usize,
    pub required_name_bytes: usize,
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
    /// The named icon walk's literal and fallback key bytes.
    pub required_name_bytes: usize,
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
    width: Option<i64>,
    token: u32,
    token_failure: Option<&'static str>,
    local: bool,
    local_failure: Option<&'static str>,
    /// [`subunit::template_category`]'s value, read with the identity.
    category: i32,
    category_failure: Option<&'static str>,
}

impl Identity {
    /// The definition's template category, once its token authenticated.
    fn category(self) -> Result<i32, &'static str> {
        self.token()?;
        match self.category_failure {
            Some(reason) => Err(reason),
            None => Ok(self.category),
        }
    }

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
            width: self.width,
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

/// A definition's identity, database index and template category bytes, in
/// the order a failed field stops the gather.
const DEFINITION_FIELDS: [RecordField; 4] = [
    record_field(0, 0, 17),
    record_field(subunit::DATABASE_INDEX, 20, 4),
    record_field(
        subunit::CATEGORY,
        DEFINITION_CATEGORY,
        subunit::CATEGORY_LEN,
    ),
    record_field(subunit::COMBAT_WIDTH, 40, 8),
];
const DEFINITION_CATEGORY: usize = 24;
const DEFINITION_RECORD: usize = 48;
const DEFINITION_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DEFINITION_FIELDS, DEFINITION_RECORD).expect("valid definition layout");

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

    /// One gathered read of a definition's identity, database index and
    /// template category. The token does not depend on the others being
    /// readable.
    fn fetch(&self, memory: &impl Memory, base: usize, at: usize) -> Identity {
        let mut record = [0_u8; DEFINITION_RECORD];
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
        let (category, category_failure) = if completed < 3 {
            (0, Some("subunit category unreadable"))
        } else {
            let bytes = record[DEFINITION_CATEGORY..DEFINITION_CATEGORY + subunit::CATEGORY_LEN]
                .try_into()
                .unwrap();
            (subunit::category_of(&bytes), None)
        };
        let token = match authenticate(base, &record[..17]) {
            Err(reason) => {
                return Identity {
                    token_failure: Some(reason),
                    ..Identity::default()
                };
            }
            // The null object has no clone flag to read.
            Ok(None) => {
                return Identity {
                    width: (completed == DEFINITION_FIELDS.len())
                        .then(|| i64::from_le_bytes(record[40..48].try_into().unwrap())),
                    category,
                    category_failure,
                    ..Identity::default()
                };
            }
            Ok(Some(token)) => token,
        };
        let local = if completed < 2 {
            Err("subunit database index unreadable")
        } else {
            self.entry(memory, dword(&record, 20))
                .map(|global| global != at)
        };
        Identity {
            width: (completed == DEFINITION_FIELDS.len())
                .then(|| i64::from_le_bytes(record[40..48].try_into().unwrap())),
            token,
            token_failure: None,
            local: local.unwrap_or(false),
            local_failure: local.err(),
            category,
            category_failure,
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

impl Definitions<'_> {
    /// A definition's template category ([`subunit::template_category`]),
    /// read with its identity once a walk.
    fn category(
        &mut self,
        memory: &impl Memory,
        base: usize,
        at: usize,
    ) -> Result<i32, &'static str> {
        self.lookup(memory, base, at).category()
    }
}

/// The fallback when the priority unit is null: regiment cell (0,0) when the low-16-bit real-unit
/// count is nonzero, else the null singleton. The native wrap is kept.
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

/// The composition's priority unit through the unit-type override: one span holding every fixed
/// scalar and the model entity string's header.
const TAIL: usize = PRIORITY_UNIT;
const TAIL_END: usize = UNIT_TYPE_OVERRIDE + 4;
/// Daily tail after the identity and the two MSVC string headers.
const TAIL_AT: usize = 80;
const RECORD: usize = TAIL_AT + TAIL_END - TAIL;
/// The daily row's fixed fields: identity, name header and the tail.
const DAILY_FIELDS: [RecordField; 3] = [
    record_field(0, 0, 16),
    record_field(NAME, 16, msvc_string::STRIDE * 2),
    record_field(TAIL, TAIL_AT, TAIL_END - TAIL),
];
const DAILY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DAILY_FIELDS, RECORD).expect("valid template layout");
/// The hourly row's fixed fields: identity, then one contiguous span from the
/// regiment grid's header through the unit-type override, which holds the
/// three grids' and the grouped entries' headers, the representative, the
/// icon fields, the country operands, the model category, the model entity
/// string and the dictionary's unit-type operands.
const ICON_SPAN: usize = GRID_DIMENSIONS;
const ICON_SPAN_END: usize = EQUIPMENT_NICHE + 1;
const _: () = assert!(MODEL_ENTITY + msvc_string::STRIDE == UNIT_TYPE_OVERRIDE);
const _: () = assert!(
    NAME + msvc_string::STRIDE <= ICON_SPAN
        && REGIMENT_DATA < SUPPORT_DATA
        && SUPPORT_DATA < REGIMENTAL_SUPPORT_DATA
        && REGIMENTAL_SUPPORT_DATA + 16 <= SUBUNIT_COUNTS
);
const ICON_RECORD: usize = 16 + ICON_SPAN_END - ICON_SPAN;
const ICON_FIELDS: [RecordField; 2] = [
    record_field(0, 0, 16),
    // Ordinary hourly acquisition retains its original fixed span. Extension
    // prefix/tail bytes are read only for templates the caller marks dirty.
    record_field(
        REGIMENT_DATA,
        16 + REGIMENT_DATA - ICON_SPAN,
        UNIT_TYPE_OVERRIDE + 4 - REGIMENT_DATA,
    ),
];
const ICON_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ICON_FIELDS, ICON_RECORD).expect("valid template icon layout");
/// The named walk's record: the icon record, then the display literal's
/// header, which lies between the identity and the icon span.
const ICON_NAME: usize = ICON_RECORD;
const ICON_NAMED_RECORD: usize = ICON_NAME + msvc_string::STRIDE;
const ICON_NAMED_FIELDS: [RecordField; 3] = [
    ICON_FIELDS[0],
    ICON_FIELDS[1],
    record_field(NAME, ICON_NAME, msvc_string::STRIDE),
];
const ICON_NAMED_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ICON_NAMED_FIELDS, ICON_NAMED_RECORD).expect("valid named icon layout");
/// A policy walk gathers every identity first, then the rest of each read record.
const IDENTITY: usize = 16;
const IDENTITY_FIELDS: [RecordField; 1] = [ICON_FIELDS[0]];
const IDENTITY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&IDENTITY_FIELDS, IDENTITY).expect("valid template identity layout");
const ICON_REST_FIELDS: [RecordField; 1] = [ICON_FIELDS[1]];
const ICON_REST_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ICON_REST_FIELDS, ICON_RECORD).expect("valid icon rest layout");
const ICON_NAMED_REST_FIELDS: [RecordField; 2] = [ICON_NAMED_FIELDS[1], ICON_NAMED_FIELDS[2]];
const ICON_NAMED_REST_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ICON_NAMED_REST_FIELDS, ICON_NAMED_RECORD)
        .expect("valid named icon rest layout");
/// The largest record, for the batch buffer.
const MAX_RECORD: usize = if RECORD > ICON_NAMED_RECORD {
    RECORD
} else {
    ICON_NAMED_RECORD
};

/// An icon-span field's place in the icon record.
const fn icon(offset: usize) -> usize {
    16 + offset - ICON_SPAN
}
/// Fixed-record gather width, also used for caller-owned batch certificates.
pub const READ_BATCH: usize = 32;
const BATCH: usize = READ_BATCH;

/// What a policy walk acquires for one authenticated roster occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acquire {
    /// Publish [`DetailPolicy::held`]; nothing more of the template is read.
    Held,
    /// Read the template, with its extension operands when `details`.
    Read { details: bool },
}

/// Selects each occurrence's reads. Country preparation precedes its roster
/// header; batch preparation precedes every identity, and the demand sees the
/// authenticated identity before anything else of the template is read.
pub trait DetailPolicy {
    fn before_country(&mut self, _country: u32) {}
    fn before_records(&mut self, _templates: &[usize]) {}
    fn demand(&mut self, country: u32, pointer: usize, template: u64, ordinal: u32) -> Acquire;
    /// The row [`Acquire::Held`] named.
    fn held(&mut self, _template: u64) -> Option<IconCore<'_>> {
        None
    }
    /// A read row, after every read of its template.
    fn remember(
        &mut self,
        _row: &IconRow,
        _bytes: &[u8],
        _counts: &[SubunitCount],
        _variants: &[u64],
        _names: &[u8],
    ) {
    }
}

/// A held row. Its spans are relative to these slices, its display name and
/// key to `names`; extensions are not held here.
pub struct IconCore<'a> {
    pub row: IconRow,
    pub bytes: &'a [u8],
    pub counts: &'a [SubunitCount],
    pub variants: &'a [u64],
    pub names: &'a [u8],
}
impl<F: FnMut(u32, usize, u64, u32) -> bool> DetailPolicy for F {
    fn demand(&mut self, country: u32, pointer: usize, template: u64, ordinal: u32) -> Acquire {
        Acquire::Read {
            details: self(country, pointer, template, ordinal),
        }
    }
}

/// How a walk reaches one authenticated occurrence.
enum Visit<'a> {
    /// The policy's held row, by handle.
    Held(u64),
    /// The gathered record, and whether extension operands are demanded.
    Read(&'a [u8], bool),
}

fn authentic(record: &[u8], vtable: u64) -> Result<(), &'static str> {
    if word(record, 0) != vtable || word(record, 8) as u32 != REFERENCE_TYPE {
        return Err("division template vtable or reference type mismatch");
    }
    Ok(())
}

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
/// owner's id, the template and its authenticated record, or the handle the
/// policy holds. Hints read nothing, so a batch's cache misses overlap instead
/// of following one another. With a policy, the batch's identities are gathered
/// first and only the occurrences it does not hold gather `rest`, the rest of
/// `layout`.
#[allow(clippy::too_many_arguments)]
fn visit_templates(
    memory: &impl Memory,
    base: usize,
    state: usize,
    layout: &RecordLayout<'_>,
    mut country_visit: Option<&mut dyn FnMut(ccountry::Country)>,
    mut details: Option<(&mut dyn DetailPolicy, &RecordLayout<'_>)>,
    mut prepare: impl FnMut(&[u8], usize),
    mut visit: impl FnMut(
        u32,
        usize,
        Visit<'_>,
        Option<&mut dyn DetailPolicy>,
        u32,
    ) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let vtable = base
        .checked_add(VTABLE_RVA)
        .ok_or("division template module address overflow")? as u64;
    let mut found_country = false;
    let mut templates = [0_usize; BATCH];
    let stride = layout.stride();
    let mut records = [0_u8; BATCH * MAX_RECORD];
    let mut identities = [0_u8; BATCH * IDENTITY];
    let mut acquired = [Acquire::Read { details: true }; BATCH];
    let mut read = [0_usize; BATCH];
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        found_country = true;
        if let Some((details, _)) = details.as_mut() {
            details.before_country(country.id);
        }
        if let Some(visit) = country_visit.as_mut() {
            visit(country);
        }
        let root = country
            .pointer
            .checked_add(ccountry::DIVISION_TEMPLATES)
            .ok_or("template roster overflow")?;
        let roster = Array::read(memory, root, 8).ok_or("template roster invalid")?;
        for first in (0..roster.count).step_by(BATCH) {
            let batch = ((roster.count - first) as usize).min(BATCH);
            pointers(memory, roster, first, &mut templates[..batch])?;
            let templates = &templates[..batch];
            let (fields, reads) = match details.as_mut() {
                None => {
                    read[..batch].copy_from_slice(templates);
                    (layout, batch)
                }
                Some((policy, rest)) => {
                    policy.before_records(templates);
                    for &at in templates {
                        memory.prefetch(at, IDENTITY);
                    }
                    if memory.copy_records(
                        RecordSources::Pointers(templates),
                        &IDENTITY_LAYOUT,
                        &mut identities[..batch * IDENTITY],
                    ) < batch
                    {
                        return Err("template fields unreadable");
                    }
                    let mut reads = 0;
                    for (index, (&at, identity)) in templates
                        .iter()
                        .zip(identities.chunks_exact(IDENTITY))
                        .enumerate()
                    {
                        authentic(identity, vtable)?;
                        acquired[index] =
                            policy.demand(country.id, at, word(identity, 8), first + index as u32);
                        if acquired[index] != Acquire::Held {
                            records[reads * stride..][..IDENTITY].copy_from_slice(identity);
                            read[reads] = at;
                            reads += 1;
                        }
                    }
                    (*rest, reads)
                }
            };
            let read = &read[..reads];
            for &at in read {
                for field in fields.fields() {
                    memory.prefetch(at.wrapping_add(field.source_offset), field.len);
                }
            }
            if !read.is_empty()
                && memory.copy_records(
                    RecordSources::Pointers(read),
                    fields,
                    &mut records[..reads * stride],
                ) < reads * fields.field_count()
            {
                return Err("template fields unreadable");
            }
            prepare(&records[..reads * stride], stride);
            let mut gathered = records[..reads * stride].chunks_exact(stride);
            for (index, &at) in templates.iter().enumerate() {
                let ordinal = first + index as u32;
                let acquire = if details.is_some() {
                    acquired[index]
                } else {
                    Acquire::Read { details: true }
                };
                let visited = match acquire {
                    Acquire::Held => Visit::Held(word(&identities[index * IDENTITY..], 8)),
                    Acquire::Read { details: demanded } => {
                        let record = gathered.next().ok_or("template fields unreadable")?;
                        authentic(record, vtable)?;
                        Visit::Read(record, demanded)
                    }
                };
                let policy = details
                    .as_mut()
                    .map(|(policy, _)| &mut **policy as &mut dyn DetailPolicy);
                visit(country.id, at, visited, policy, ordinal)?;
            }
        }
    }
    if !found_country {
        return Err("template census contains no countries");
    }
    Ok(())
}

/// Priority subunit or its fallback.
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
/// and the unit_type category. Capture derives these hourly from
/// [`walk_icons`] and [`copy_names_growing`]; this full walk sizes them.
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
    walk_inner(
        memory,
        base,
        state,
        Buffer::Fixed(rows),
        Buffer::Fixed(bytes),
        memo,
    )
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut rows: Buffer<'_, Row>,
    mut bytes: Buffer<'_, u8>,
    memo: &mut [MemoSlot],
) -> Walk {
    let mut result = Walk {
        outcome: Outcome::Rows { len: 0 },
        bytes_len: 0,
        required_bytes: 0,
        required_name_bytes: 0,
        required_memo: 0,
    };
    let mut memo = Memo::new(memo);
    let (mut len, mut missing) = (0, 0_u32);
    let captured = visit_templates(
        memory,
        base,
        state,
        &DAILY_LAYOUT,
        None,
        None,
        |_, _| {},
        |owner, at, visited, _, _| {
            let Visit::Read(record, _) = visited else {
                unreachable!("the daily walk has no policy to hold a row")
            };
            let name = Descriptor::from_header(
                memory,
                at.checked_add(NAME)
                    .ok_or("template name address overflow")?,
                record[16..48].try_into().unwrap(),
            )?;
            let name_key = Descriptor::from_header(
                memory,
                at.checked_add(NAME_KEY)
                    .ok_or("template name key address overflow")?,
                record[48..80].try_into().unwrap(),
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
                .and_then(|n| n.checked_add(name_key.len))
                .ok_or("template byte arena overflow")?;
            result.required_name_bytes = result
                .required_name_bytes
                .checked_add(name.len)
                .and_then(|n| n.checked_add(name_key.len))
                .ok_or("template display name arena overflow")?;
            let end = result
                .bytes_len
                .checked_add(name.len)
                .and_then(|n| n.checked_add(name_key.len))
                .ok_or("template byte arena overflow")?;
            if !rows.ensure(len + 1)? || (matches!(bytes, Buffer::Fixed(_)) && end > bytes.len()) {
                missing = missing
                    .checked_add(1)
                    .ok_or("template row count overflow")?;
                return Ok(());
            }
            let start = result.bytes_len;
            name.copy_buffer(memory, &mut bytes, start)?;
            result.bytes_len += name.len;
            let key_start = result.bytes_len;
            name_key.copy_buffer(memory, &mut bytes, key_start)?;
            result.bytes_len += name_key.len;
            rows[len] = Row {
                template: word(record, 8),
                owner,
                name: Span {
                    start,
                    len: name.len,
                },
                name_key: Span {
                    start: key_start,
                    len: name_key.len,
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

/// A template's literal name and fallback key, copied as [`walk`] copies them into `bytes` from
/// `start`, for a caller holding the rest of its dictionary row from this hour's [`walk_icons`]
/// ([`IconRow`]).
pub fn copy_names_growing(
    memory: &impl Memory,
    template: usize,
    bytes: &mut Vec<u8>,
    start: usize,
) -> Result<[Span; 2], &'static str> {
    let mut spans = [Span::default(); 2];
    let mut at = start;
    for (span, offset) in spans.iter_mut().zip([NAME, NAME_KEY]) {
        let name = Descriptor::read(
            memory,
            template
                .checked_add(offset)
                .ok_or("template name address overflow")?,
        )?;
        name.copy_buffer(memory, &mut Buffer::Growing(bytes), at)?;
        *span = Span {
            start: at,
            len: name.len,
        };
        at += name.len;
    }
    Ok(spans)
}

/// Every country template's hourly inputs ([`IconRow`]), roster by roster in
/// the order [`walk`] visits them: what the counter draws and the stored
/// results the 3D model selectors read (the representative, the grouped
/// (definition, count) entries and the saved model choices), and each of the
/// three grids' (definition, cell count) entries. A player's edit reaches all
/// of them within the hour. Capacity omits a WHOLE row, as in [`walk`]; a
/// row's grid entries are budgeted at one per cell. A fault in a template's
/// model inputs (its grouped entries, grids, exclusions, model string or a
/// definition's clone flag) fails only that row's model fields, so every
/// counter keeps its icon; a sub-unit database too large for its slots is
/// read live and the slots grow.
/// Each roster batch is gathered in two spans a template (identity and one
/// contiguous span from the grid headers through the model entity), the
/// grouped entries and each grid's cells in one more each, and definitions
/// resolve through `Definitions`.
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
    walk_icons_inner(
        memory,
        base,
        state,
        Buffer::Fixed(rows),
        Buffer::Fixed(bytes),
        Buffer::Fixed(counts),
        Buffer::Fixed(variants),
        slots,
        None,
        None,
        None,
    )
}

/// Same authenticated hourly roster, with each template's exact display name;
/// its literal's header comes with the record. A text fault or shortage
/// refuses only the optional name, never counter/model facts. The caller grows
/// the separate name arena off-thread from observed demand.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub fn walk_icons_with_names(
    memory: &impl Memory,
    base: usize,
    state: usize,
    rows: &mut [IconRow],
    bytes: &mut [u8],
    counts: &mut [SubunitCount],
    variants: &mut [u64],
    slots: DefinitionSlots<'_>,
    names: &mut [u8],
) -> IconWalk {
    walk_icons_inner(
        memory,
        base,
        state,
        Buffer::Fixed(rows),
        Buffer::Fixed(bytes),
        Buffer::Fixed(counts),
        Buffer::Fixed(variants),
        slots,
        Some(Buffer::Fixed(names)),
        None,
        None,
    )
}

/// Worker-only acquisition grows each arena from the already acquired record.
#[allow(clippy::too_many_arguments)]
pub fn walk_icons_with_names_growing(
    memory: &impl Memory,
    base: usize,
    state: usize,
    rows: &mut Vec<IconRow>,
    bytes: &mut Vec<u8>,
    counts: &mut Vec<SubunitCount>,
    variants: &mut Vec<u64>,
    slots: DefinitionSlots<'_>,
    names: &mut Vec<u8>,
) -> IconWalk {
    walk_icons_inner(
        memory,
        base,
        state,
        Buffer::Growing(rows),
        Buffer::Growing(bytes),
        Buffer::Growing(counts),
        Buffer::Growing(variants),
        slots,
        Some(Buffer::Growing(names)),
        None,
        None,
    )
}

/// The existing named/country census with its reads selected per template.
/// A held occurrence reads nothing past its identity; a read without details
/// performs no extension prefix/tail or forbidden-type payload read.
#[allow(clippy::too_many_arguments)]
pub fn walk_icons_with_selected_details_growing(
    memory: &impl Memory,
    base: usize,
    state: usize,
    rows: &mut Vec<IconRow>,
    bytes: &mut Vec<u8>,
    counts: &mut Vec<SubunitCount>,
    variants: &mut Vec<u64>,
    slots: DefinitionSlots<'_>,
    names: &mut Vec<u8>,
    countries: &mut dyn FnMut(ccountry::Country),
    details: &mut dyn DetailPolicy,
) -> IconWalk {
    walk_icons_inner(
        memory,
        base,
        state,
        Buffer::Growing(rows),
        Buffer::Growing(bytes),
        Buffer::Growing(counts),
        Buffer::Growing(variants),
        slots,
        Some(Buffer::Growing(names)),
        Some(countries),
        Some(details),
    )
}

/// Advisory capacity from the existing icon sources. Never reads extension
/// prefix/tail bytes or forbidden-equipment payloads to size a future capture.
pub fn icon_capacity(memory: &impl Memory, base: usize, state: usize) -> IconWalk {
    walk_icons_inner(
        memory,
        base,
        state,
        Buffer::Fixed(&mut []),
        Buffer::Fixed(&mut []),
        Buffer::Fixed(&mut []),
        Buffer::Fixed(&mut []),
        DefinitionSlots {
            database: &mut [],
            globals: &mut [],
            locals: &mut [],
        },
        None,
        None,
        Some(&mut |_: u32, _: usize, _: u64, _: u32| false),
    )
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DisplayName {
    pub literal: Span,
    /// None when the nonempty literal makes the fallback not demanded.
    pub fallback_key: Option<Span>,
}

/// Called only after the exact wrapper authenticates in this world read.
/// Display naming selects literal text and fallback key from that wrapper;
/// the current icon country already carries its selector.
#[cfg(test)]
fn display_name(
    memory: &impl Memory,
    wrapper: usize,
    bytes: &mut [u8],
    used: &mut usize,
    required: &mut usize,
) -> Result<DisplayName, &'static str> {
    let header = wrapper
        .checked_add(NAME)
        .and_then(|at| read::<[u8; msvc_string::STRIDE]>(memory, at))
        .ok_or("string header unreadable")?;
    display_name_buffer(
        memory,
        wrapper,
        header,
        &mut Buffer::Fixed(bytes),
        used,
        required,
    )
}

/// `literal` is the `NAME` header the named walk gathered with the record.
fn display_name_buffer(
    memory: &impl Memory,
    wrapper: usize,
    literal: [u8; msvc_string::STRIDE],
    bytes: &mut Buffer<'_, u8>,
    used: &mut usize,
    required: &mut usize,
) -> Result<DisplayName, &'static str> {
    let literal = Descriptor::from_header(
        memory,
        wrapper
            .checked_add(NAME)
            .ok_or("template display literal address overflow")?,
        literal,
    )?;
    let key = if literal.len == 0 {
        Some(Descriptor::read(
            memory,
            wrapper
                .checked_add(NAME_KEY)
                .ok_or("template display key address overflow")?,
        )?)
    } else {
        None
    };
    let demand = literal
        .len
        .checked_add(key.as_ref().map_or(0, |key| key.len))
        .ok_or("template display name arena overflow")?;
    *required = required
        .checked_add(demand)
        .ok_or("template display name demand overflow")?;
    let start = *used;
    let end = start
        .checked_add(demand)
        .ok_or("template display name arena overflow")?;
    if matches!(bytes, Buffer::Fixed(_)) && end > bytes.len() {
        return Err("template display name arena shortage");
    }
    literal.copy_buffer(memory, bytes, start)?;
    if let Some(key) = &key {
        key.copy_buffer(memory, bytes, start + literal.len)?;
    }
    // Neither partial copy nor invalid descriptor publishes any span or length.
    *used = end;
    Ok(DisplayName {
        literal: Span {
            start,
            len: literal.len,
        },
        fallback_key: key.map(|key| Span {
            start: start + literal.len,
            len: key.len,
        }),
    })
}

/// The fallback key behind a nonempty literal, for the dictionary; failure
/// publishes no span or length.
fn name_key_buffer(
    memory: &impl Memory,
    wrapper: usize,
    bytes: &mut Buffer<'_, u8>,
    used: &mut usize,
    required: &mut usize,
) -> Result<Span, &'static str> {
    let key = Descriptor::read(
        memory,
        wrapper
            .checked_add(NAME_KEY)
            .ok_or("template name key address overflow")?,
    )?;
    *required = required
        .checked_add(key.len)
        .ok_or("template display name demand overflow")?;
    let start = *used;
    let end = start
        .checked_add(key.len)
        .ok_or("template display name arena overflow")?;
    if matches!(bytes, Buffer::Fixed(_)) && end > bytes.len() {
        return Err("template display name arena shortage");
    }
    key.copy_buffer(memory, bytes, start)?;
    *used = end;
    Ok(Span {
        start,
        len: key.len,
    })
}

/// The validated name descriptors of one walk's priority sub-units, keyed by
/// definition: a few definitions are every template's priority unit, so each
/// name's header and terminator are read once a walk and only its bytes per
/// template. A slot another definition took only costs a fresh read.
struct SubunitNames([(usize, Option<Descriptor>); SUBUNIT_NAMES]);
const SUBUNIT_NAMES: usize = 64;

impl SubunitNames {
    fn new() -> Self {
        Self([(0, None); SUBUNIT_NAMES])
    }

    fn read(&mut self, memory: &impl Memory, at: usize) -> Result<Descriptor, &'static str> {
        let hash = (at as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let slot = &mut self.0[(hash >> (64 - SUBUNIT_NAMES.trailing_zeros())) as usize];
        if slot.0 == at
            && let Some(name) = slot.1
        {
            return Ok(name);
        }
        let name = Descriptor::read(
            memory,
            at.checked_add(subunit::NAME)
                .ok_or("subunit name overflow")?,
        )?;
        *slot = (at, Some(name));
        Ok(name)
    }
}

/// A template's model-input headers, decoded from its gathered record.
struct Headers {
    grouped: Array,
    grids: [Array; 3],
    /// The grids' cells together, the most entries they can need.
    cells: usize,
    exclusions: Array,
    model_entity: Descriptor,
}

#[allow(clippy::too_many_arguments)]
fn walk_icons_inner(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut rows: Buffer<'_, IconRow>,
    mut bytes: Buffer<'_, u8>,
    mut counts: Buffer<'_, SubunitCount>,
    mut variants: Buffer<'_, u64>,
    slots: DefinitionSlots<'_>,
    mut display_bytes: Option<Buffer<'_, u8>>,
    country_visit: Option<&mut dyn FnMut(ccountry::Country)>,
    detail_policy: Option<&mut dyn DetailPolicy>,
) -> IconWalk {
    let mut result = IconWalk {
        outcome: Outcome::Rows { len: 0 },
        name_bytes_len: 0,
        required_name_bytes: 0,
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
    let mut cell_scratch = [0_u8; BATCH * 8];
    let mut excluded_scratch = [0_usize; BATCH];
    // A fully carried walk needs no definition database or scratch reset.
    let mut slots = Some(slots);
    let mut memo: Option<Definitions<'_>> = None;
    let mut subunit_names = SubunitNames::new();
    let hint = |records: &[u8], stride: usize| {
        for record in records.chunks_exact(stride) {
            for (header, stride) in [
                (icon(SUBUNIT_COUNTS), SUBUNIT_COUNT_STRIDE),
                (icon(EXCLUDED_VARIANT_DATA), 8),
                (icon(GRIDS[0]), 8),
                (icon(GRIDS[1]), 8),
                (icon(GRIDS[2]), 8),
            ] {
                let count =
                    u32::from_le_bytes(record[header + 12..header + 16].try_into().unwrap());
                let data = word(record, header) as usize;
                memory.prefetch(data, (count as usize).saturating_mul(stride));
            }
        }
    };
    let (layout, rest) = match display_bytes {
        Some(_) => (&ICON_NAMED_LAYOUT, &ICON_NAMED_REST_LAYOUT),
        None => (&ICON_LAYOUT, &ICON_REST_LAYOUT),
    };
    let captured = visit_templates(
        memory,
        base,
        state,
        layout,
        country_visit,
        detail_policy.map(|policy| (policy, rest)),
        hint,
        |country, at, visited, mut policy, roster_ordinal| {
            let (record, mut details_fresh) = match visited {
                Visit::Read(record, details) => (record, details),
                Visit::Held(template) => {
                    let Some(core) = policy.as_mut().and_then(|policy| policy.held(template))
                    else {
                        return Err("template held row unavailable");
                    };
                    let byte_start = result.bytes_len;
                    let count_start = result.counts_len;
                    let variant_start = result.variants_len;
                    let byte_end = byte_start
                        .checked_add(core.bytes.len())
                        .ok_or("template byte arena overflow")?;
                    let count_end = count_start
                        .checked_add(core.counts.len())
                        .ok_or("template grouped arena overflow")?;
                    let variant_end = variant_start
                        .checked_add(core.variants.len())
                        .ok_or("template variant arena overflow")?;
                    result.required_bytes = result
                        .required_bytes
                        .checked_add(core.bytes.len())
                        .ok_or("template byte arena overflow")?;
                    result.required_counts = result
                        .required_counts
                        .checked_add(core.counts.len())
                        .ok_or("template grouped arena overflow")?;
                    result.required_variants = result
                        .required_variants
                        .checked_add(core.variants.len())
                        .ok_or("template variant arena overflow")?;
                    if !rows.ensure(len + 1)?
                        || !bytes.ensure(byte_end)?
                        || !counts.ensure(count_end)?
                        || !variants.ensure(variant_end)?
                    {
                        missing = missing
                            .checked_add(1)
                            .ok_or("template row count overflow")?;
                        return Ok(());
                    }
                    bytes[byte_start..byte_end].copy_from_slice(core.bytes);
                    counts[count_start..count_end].copy_from_slice(core.counts);
                    variants[variant_start..variant_end].copy_from_slice(core.variants);
                    let mut row = core.row;
                    row.priority_unit_name.start += byte_start;
                    row.model_entity.start += byte_start;
                    row.counts.start += count_start;
                    for grid in &mut row.grids {
                        grid.start += count_start;
                    }
                    row.excluded_variants.start += variant_start;
                    row.settings.roster_ordinal = roster_ordinal;
                    result.bytes_len = byte_end;
                    result.counts_len = count_end;
                    result.variants_len = variant_end;
                    let name_start = result.name_bytes_len;
                    let name_end = name_start
                        .checked_add(core.names.len())
                        .ok_or("template display name arena overflow")?;
                    let held_names = match display_bytes.as_mut() {
                        Some(names) => {
                            result.required_name_bytes = result
                                .required_name_bytes
                                .checked_add(core.names.len())
                                .ok_or("template display name demand overflow")?;
                            let fits = names.ensure(name_end)?;
                            if fits {
                                names[name_start..name_end].copy_from_slice(core.names);
                            }
                            fits
                        }
                        None => false,
                    };
                    if held_names {
                        result.name_bytes_len = name_end;
                        if let Some(name) = &mut row.display_name {
                            name.literal.start += name_start;
                            if let Some(key) = &mut name.fallback_key {
                                key.start += name_start;
                            }
                        }
                        if let Some(key) = &mut row.name_key {
                            key.start += name_start;
                        }
                    } else {
                        row.display_name = None;
                        row.name_key = None;
                    }
                    rows[len] = row;
                    len += 1;
                    return Ok(());
                }
            };
            let mut details_failure = None;
            let mut extension = [0_u8; ICON_NAMED_RECORD];
            extension[..record.len()].copy_from_slice(record);
            if details_fresh {
                let mut dimensions = [0_u8; REGIMENT_DATA - GRID_DIMENSIONS];
                let mut tail = [0_u8; ICON_SPAN_END - (UNIT_TYPE_OVERRIDE + 4)];
                if memory.copy_scattered(
                    [
                        at.checked_add(GRID_DIMENSIONS)
                            .ok_or("template dimensions address overflow")?,
                        at.checked_add(UNIT_TYPE_OVERRIDE + 4)
                            .ok_or("template settings address overflow")?,
                    ],
                    &mut [&mut dimensions, &mut tail],
                ) != 2
                {
                    details_fresh = false;
                    details_failure = Some("template extension operands unreadable");
                } else {
                    extension[icon(GRID_DIMENSIONS)..icon(REGIMENT_DATA)]
                        .copy_from_slice(&dimensions);
                    extension[icon(UNIT_TYPE_OVERRIDE + 4)..ICON_RECORD].copy_from_slice(&tail);
                }
            }
            let record = &extension[..record.len()];
            let memo = memo.get_or_insert_with(|| {
                Definitions::load(
                    memory,
                    base,
                    slots.take().expect("first definition demand"),
                    &mut result.required_database,
                )
            });
            if owner != Some(country) {
                owner = Some(country);
                memo.begin_country();
            }
            // Counter drawing operands; a failure here fails the hour's table.
            let selected =
                selected_unit(memory, base, memo, at, word(record, icon(PRIORITY_UNIT)))?;
            let priority_unit = memo.token(memory, base, selected)?;
            // The dictionary's unit_type, as `walk` derives it.
            let unit_type = match dword(record, icon(UNIT_TYPE_OVERRIDE)) {
                0 => match word(record, icon(CATEGORY_UNIT)) as usize {
                    0 => memo.category(memory, base, selected),
                    category => memo.category(memory, base, category),
                },
                stored => Ok(stored),
            };
            let selected_name = if priority_unit != 0 {
                Some(subunit_names.read(memory, selected)?)
            } else {
                None
            };
            // The model inputs' headers, read with the record. Their demand is
            // counted before any capacity check, so every arena grows at once.
            let headers = (|| -> Result<Headers, &'static str> {
                let header = |at: usize| -> [u8; 16] { record[at..at + 16].try_into().unwrap() };
                let grouped =
                    Array::from_header(header(icon(SUBUNIT_COUNTS)), SUBUNIT_COUNT_STRIDE)
                        .ok_or("template grouped subunits invalid")?;
                // A grid's entries are at most its cells.
                let mut grids = [Array::default(); 3];
                let mut cells = 0_usize;
                for (grid, offset) in grids.iter_mut().zip(GRIDS) {
                    *grid = Array::from_header(header(icon(offset)), 8)
                        .ok_or("template grid invalid")?;
                    cells = cells
                        .checked_add(grid.count as usize)
                        .ok_or("template grouped arena overflow")?;
                }
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
                    .and_then(|required| {
                        required.checked_add(cells.checked_mul(if details_fresh {
                            2
                        } else {
                            1
                        })?)
                    })
                    .ok_or("template grouped arena overflow")?;
                result.required_variants = result
                    .required_variants
                    .checked_add(exclusions.count as usize)
                    .ok_or("template variant arena overflow")?;
                Ok(Headers {
                    grouped,
                    grids,
                    cells,
                    exclusions,
                    model_entity,
                })
            })();
            let forbidden = if details_fresh {
                Array::from_header(
                    record[icon(FORBIDDEN_EQUIPMENT)..icon(FORBIDDEN_EQUIPMENT) + 16]
                        .try_into()
                        .unwrap(),
                    8,
                )
                .ok_or("template forbidden equipment invalid")
            } else {
                Ok(Array::default())
            };
            if let Ok(array) = forbidden {
                result.required_variants = result
                    .required_variants
                    .checked_add(array.count as usize)
                    .ok_or("template variant arena overflow")?;
            }
            let name_len = selected_name.as_ref().map_or(0, |name| name.len);
            result.required_bytes = result
                .required_bytes
                .checked_add(name_len)
                .ok_or("template byte arena overflow")?;
            let name_end = result
                .bytes_len
                .checked_add(name_len)
                .ok_or("template byte arena overflow")?;
            if !rows.ensure(len + 1)?
                || (matches!(bytes, Buffer::Fixed(_)) && name_end > bytes.len())
            {
                missing = missing
                    .checked_add(1)
                    .ok_or("template row count overflow")?;
                return Ok(());
            }
            let row_start = (result.bytes_len, result.counts_len, result.variants_len);
            let mut row = IconRow {
                manpower_exile: dword(record, icon(ORIGIN_TYPE))
                    == crate::carmy_combat_inputs::EXILE_ORIGIN,
                template: word(record, 8),
                pointer: at,
                owner: country,
                unit_type: *unit_type.as_ref().unwrap_or(&0),
                unit_type_failure: unit_type.err(),
                priority_unit,
                priority_unit_name: Span {
                    start: result.bytes_len,
                    len: name_len,
                },
                custom_icon: dword(record, icon(CUSTOM_ICON)),
                custom_icon_nato: record[icon(CUSTOM_ICON_NATO)],
                formation_division_count: Some(dword(record, icon(FORMATION_DIVISION_COUNT))),
                grid_dimensions: GRIDS.map(|grid| dword(record, icon(grid - 8)) as u32),
                details_fresh,
                details_failure,
                settings: Settings {
                    roster_ordinal,
                    priority: dword(record, icon(PRIORITY)),
                    is_army_hq: record[icon(IS_ARMY_HQ)],
                    locked: record[icon(LOCKED)],
                    obsolete: record[icon(OBSOLETE_FLAGS)],
                    obsolete_change_date: dword(record, icon(OBSOLETE_CHANGE_DATE)),
                    force_allow_recruiting: record[icon(FORCE_ALLOW_RECRUITING)],
                    allow_new_equipment: record[icon(ALLOW_NEW_EQUIPMENT)],
                    allow_foreign_equipment: record[icon(ALLOW_FOREIGN_EQUIPMENT)],
                    parent: word(record, icon(PARENT)),
                    creator_country: dword(record, icon(CREATOR_COUNTRY)),
                    origin_type: dword(record, icon(ORIGIN_TYPE)),
                    fake_intel: record[icon(FAKE_INTEL)],
                    division_cap: (record[icon(DIVISION_CAP_PRESENT)] != 0)
                        .then(|| dword(record, icon(DIVISION_CAP))),
                    equipment_niche: record[icon(EQUIPMENT_NICHE)],
                    role: (record[icon(ROLE_PRESENT)] != 0)
                        .then(|| dword(record, icon(ROLE)) as u32),
                    ai_role: (record[icon(AI_ROLE_PRESENT)] != 0)
                        .then(|| dword(record, icon(AI_ROLE)) as u32),
                    ai_role_fallback: dword(record, icon(AI_ROLE_FALLBACK)) as u32,
                    total_manpower: dword(record, icon(TOTAL_MANPOWER)),
                    command_power_cost: word(record, icon(COMMAND_POWER_COST)) as i64,
                    training_time: dword(record, icon(TRAINING_TIME)),
                },
                // Read with the authenticated record, so they cannot fail.
                country: dword(record, icon(COUNTRY_OPERAND_289A)),
                foreign_country: dword(record, icon(COUNTRY_OPERAND_3ABE)),
                // The optional's inactive payload is not an input.
                model_category: (record[icon(MODEL_CATEGORY_PRESENT)] != 0)
                    .then(|| word(record, icon(MODEL_CATEGORY))),
                ..IconRow::default()
            };
            if let Some(name) = &selected_name {
                name.copy_buffer(memory, &mut bytes, result.bytes_len)?;
                result.bytes_len += name.len;
            }
            // The 3D model's inputs. A fault here fails only this row's model
            // fields; the counter keeps its icon. `Ok(false)` is a row that does
            // not fit, omitted whole.
            let model = (|| -> Result<bool, &'static str> {
                let Headers {
                    grouped,
                    grids,
                    cells,
                    exclusions,
                    model_entity,
                } = headers?;
                let bytes_end = result
                    .bytes_len
                    .checked_add(model_entity.len)
                    .ok_or("template byte arena overflow")?;
                let counts_end = result
                    .counts_len
                    .checked_add(grouped.count as usize)
                    .and_then(|end| {
                        end.checked_add(cells.checked_mul(if details_fresh { 2 } else { 1 })?)
                    })
                    .ok_or("template grouped arena overflow")?;
                let variants_end = result
                    .variants_len
                    .checked_add(exclusions.count as usize)
                    .ok_or("template variant arena overflow")?;
                if (matches!(bytes, Buffer::Fixed(_)) && bytes_end > bytes.len())
                    || !counts.ensure(counts_end)?
                    || !variants.ensure(variants_end)?
                {
                    return Ok(false);
                }
                // Stored representative model, then priority fallback.
                row.representative = match word(record, icon(VISUAL_REPRESENTATIVE)) as usize {
                    0 => memo.identify(memory, base, selected)?,
                    representative => memo.identify(memory, base, representative)?,
                };
                row.model_entity = Span {
                    start: result.bytes_len,
                    len: model_entity.len,
                };
                model_entity.copy_buffer(memory, &mut bytes, result.bytes_len)?;
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
                // Each grid's definitions, counted once per occupied cell.
                let cell_bytes = &mut cell_scratch;
                for (grid_index, (span, grid)) in row.grids.iter_mut().zip(grids).enumerate() {
                    let start = result.counts_len;
                    let cell_start = start + grid.count as usize;
                    let mut cell_index = 0;
                    for first in (0..grid.count).step_by(BATCH) {
                        let len = ((grid.count - first) as usize).min(BATCH) * 8;
                        let at = grid
                            .element(first, 8)
                            .ok_or("template grid cell index invalid")?;
                        if !read_slice(memory, at, &mut cell_bytes[..len]) {
                            return Err("template grid cells unreadable");
                        }
                        for cell in cell_bytes[..len].chunks_exact(8) {
                            let cell =
                                usize::try_from(u64::from_le_bytes(cell.try_into().unwrap()))
                                    .map_err(|_| "template grid cell pointer invalid")?;
                            if cell == 0 {
                                if details_fresh {
                                    counts[cell_start + cell_index] = SubunitCount::default();
                                }
                                cell_index += 1;
                                continue;
                            }
                            let unit = memo.identify(memory, base, cell)?;
                            if details_fresh {
                                counts[cell_start + cell_index] = SubunitCount { count: 1, ..unit };
                            }
                            cell_index += 1;
                            let end = result.counts_len;
                            match counts[start..end].iter_mut().find(|entry| {
                                (entry.token, entry.local) == (unit.token, unit.local)
                            }) {
                                Some(entry) => {
                                    entry.count = entry
                                        .count
                                        .checked_add(1)
                                        .ok_or("template grid count overflow")?;
                                }
                                None => {
                                    counts[end] = SubunitCount { count: 1, ..unit };
                                    result.counts_len = end + 1;
                                }
                            }
                        }
                    }
                    *span = Span {
                        start,
                        len: result.counts_len - start,
                    };
                    if details_fresh {
                        let cell_end = cell_start + grid.count as usize;
                        let spatial_start = result.counts_len;
                        counts.copy_within(cell_start..cell_end, spatial_start);
                        row.cells[grid_index] = Span {
                            start: spatial_start,
                            len: grid.count as usize,
                        };
                        result.counts_len = spatial_start + grid.count as usize;
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
                        manpower_exile: row.manpower_exile,
                        template: row.template,
                        pointer: row.pointer,
                        owner: row.owner,
                        unit_type: row.unit_type,
                        unit_type_failure: row.unit_type_failure,
                        priority_unit: row.priority_unit,
                        priority_unit_name: row.priority_unit_name,
                        custom_icon: row.custom_icon,
                        custom_icon_nato: row.custom_icon_nato,
                        formation_division_count: row.formation_division_count,
                        grid_dimensions: row.grid_dimensions,
                        details_fresh: row.details_fresh,
                        details_failure: row.details_failure,
                        settings: row.settings,
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
                        grids: [Span {
                            start: row_start.1,
                            len: 0,
                        }; 3],
                        excluded_variants: Span {
                            start: row_start.2,
                            len: 0,
                        },
                        model_failure: Some(reason),
                        ..IconRow::default()
                    };
                }
            }
            let equipment_start = result.variants_len;
            let equipment = (|| -> Result<Span, &'static str> {
                let forbidden = forbidden?;
                let end = equipment_start
                    .checked_add(forbidden.count as usize)
                    .ok_or("template variant arena overflow")?;
                if !variants.ensure(end)? {
                    return Err("template forbidden equipment arena shortage");
                }
                let span = Span {
                    start: result.variants_len,
                    len: forbidden.count as usize,
                };
                let excluded = &mut excluded_scratch;
                for first in (0..forbidden.count).step_by(BATCH) {
                    let batch = ((forbidden.count - first) as usize).min(BATCH);
                    pointers(memory, forbidden, first, &mut excluded[..batch])?;
                    for &definition in &excluded[..batch] {
                        let identity = read::<[u8; 12]>(memory, definition)
                            .ok_or("template forbidden equipment unreadable")?;
                        let vtable = word(&identity, 0);
                        if vtable != (base + crate::cequipment_type::VTABLE_RVA) as u64
                            && vtable != (base + crate::cequipment_type::NULL_VTABLE_RVA) as u64
                        {
                            return Err("template forbidden equipment identity mismatch");
                        }
                        variants[result.variants_len] = dword(&identity, 8) as u32 as u64;
                        result.variants_len += 1;
                    }
                }
                Ok(span)
            })();
            match equipment {
                Ok(span) => row.forbidden_equipment = span,
                Err(reason) => {
                    result.variants_len = equipment_start;
                    row.equipment_failure = Some(reason);
                }
            }
            if details_fresh {
                let start = result.bytes_len;
                let group = (|| -> Result<Option<Span>, &'static str> {
                    let pointer =
                        word(record, icon(crate::cname_group_member::TEMPLATE_NAME_GROUP)) as usize;
                    if pointer == 0 {
                        return Ok(None);
                    }
                    let header = read::<[u8; 40]>(memory, pointer)
                        .ok_or("template naming group unreadable")?;
                    if word(&header, 0)
                        != (base + crate::cname_group_member::GROUP_VTABLE_RVA) as u64
                    {
                        return Err("template naming group identity mismatch");
                    }
                    let key = Descriptor::from_header(
                        memory,
                        pointer
                            .checked_add(crate::cname_group_member::GROUP_KEY)
                            .ok_or("template naming group address overflow")?,
                        header[8..40].try_into().unwrap(),
                    )?;
                    result.required_bytes = result
                        .required_bytes
                        .checked_add(key.len)
                        .ok_or("template naming group arena overflow")?;
                    key.copy_buffer(memory, &mut bytes, start)?;
                    result.bytes_len += key.len;
                    Ok(Some(Span {
                        start,
                        len: key.len,
                    }))
                })();
                match group {
                    Ok(group) => row.names_group = group,
                    Err(reason) => {
                        result.bytes_len = start;
                        row.details_fresh = false;
                        row.details_failure = Some(reason);
                    }
                }
            }
            if let Some(names) = display_bytes.as_mut() {
                row.display_name = display_name_buffer(
                    memory,
                    at,
                    record[ICON_NAME..ICON_NAME + msvc_string::STRIDE]
                        .try_into()
                        .unwrap(),
                    names,
                    &mut result.name_bytes_len,
                    &mut result.required_name_bytes,
                )
                .ok();
                row.name_key = match row.display_name {
                    None => None,
                    Some(DisplayName {
                        fallback_key: Some(key),
                        ..
                    }) => Some(key),
                    Some(_) => name_key_buffer(
                        memory,
                        at,
                        names,
                        &mut result.name_bytes_len,
                        &mut result.required_name_bytes,
                    )
                    .ok(),
                };
            }
            if let Some(policy) = policy.as_mut() {
                policy.remember(
                    &row,
                    &bytes,
                    &counts,
                    &variants,
                    display_bytes.as_deref().unwrap_or(&[]),
                );
            }
            rows[len] = row;
            len += 1;
            Ok(())
        },
    );
    result.required_locals = memo.as_ref().map_or(0, Definitions::required_locals);
    result.outcome = match captured {
        Ok(()) => outcome(len, missing),
        Err(reason) => Outcome::Unavailable { reason },
    };
    result
}

/// Authenticates a template pointer and returns its full native handle: the
/// reference type in the low word and the id in the high word. Callers that
/// accept a null pointer must test for it before calling.
pub fn identity(memory: &impl Memory, base: usize, pointer: usize) -> Result<u64, &'static str> {
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

#[cfg(test)]
#[path = "creferenced_division_template/display_name_tests.rs"]
mod display_name_tests;
