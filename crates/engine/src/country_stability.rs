//! Country operands for displayed stability, war support and maximum command
//! power, with modifier-tree children and identifiable sources. Displayed
//! values are calculated, so this reader copies operands for replay consumers
//! and never calls a game function.
//!
//! Stability combines base, war and ruling-party terms, propaganda penalty,
//! effective modifier and cached coastal contribution before loaded bounds.
//! War support combines base, offensive/defensive war terms, tension,
//! bombing/interception/hero/convoy/propaganda terms and its effective modifier
//! before loaded bounds. Maximum command power applies effective flat and
//! multiplicative terms to its loaded base.
//!
//! Modifier rebuilding appends one clone per non-empty source in source order.
//! Clones retain entries, display text, category mask and depth, with token
//! reset. For tracked good keys each child entry is its root contribution.
//! Pairing replays supported sources in order, requiring equal nonzero entries
//! and equal copied display text where applicable. Pair only when the leftmost
//! and rightmost order-preserving assignments agree; ambiguous children keep
//! display text. Only children with tracked values and sources with tracked
//! keys participate so empty signatures cannot shift bounds.
//!
//! Supported sources are ideas, the ruling party's first country leader, its
//! ideology group, appointed advisors, bound named statics, enabled dynamic
//! modifiers in constant/evaluated order, and scaled stability/war-support statics.
//! Other source kinds remain unpaired. Operands include owned war relations,
//! interception-ring sum, loaded defines, active language and tracked modifier
//! queries through `modifier_definition::keys_by_names_at`. `raid_launch_inputs`
//! independently supplies maximum-power modifiers and allocated power in hourly
//! CountryCommandPower; consumers join the same frame. Tooltip-only ideas,
//! all-zero dynamic modifiers and zero-valued faction children cannot shift
//! pairing. Fixtures cover ambiguity, drift, truncation, short reads and fingerprint.

use crate::{
    Memory, cadvisor, ccharacter, ccountry, ccountry_characters, ccountry_leader,
    cdiplomacy_status, cdynamic_modifier, cdynamic_modifier_container, cidea, cideology_group,
    cmodifier,
    cpdx_array::Array,
    cpolitical_party, cpolitical_status, crelation, cstatic_modifier, cwar_relation, cworld_threat,
    dynamic_modifier_entry, localization_root_layout, modifier_accumulator, modifier_definition,
    modifier_entry,
    msvc_string::Descriptor,
    read, read_slice,
    walk::{Buffer, Outcome},
};

/// Recorded modifier names match replay `CountryStabilityInputRow::KEY_NAMES`
/// order. Resolve each index through [`modifier_definition::keys_by_names_at`].
/// These are good percentage keys except maximum command power, a good numeric key.
pub const KEY_NAMES: [&[u8]; TRACKED] = [
    b"stability_factor",
    b"party_popularity_stability_factor",
    b"war_stability_factor",
    b"offensive_war_stability_factor",
    b"defensive_war_stability_factor",
    b"war_support_factor",
    b"stability_weekly",
    b"war_support_weekly",
    b"max_command_power",
    b"max_command_power_mult",
];
pub const TRACKED: usize = 10;
/// The keys whose totals the row records: all but `max_command_power` and
/// `max_command_power_mult`, whose getter values CountryCommandPower records
/// (one reader per value). Their lines are still read here.
pub const TOTALS: usize = 8;

/// Stored Q15 propaganda stability penalty, clamped weekly. Serialization
/// converts to fixed5 with five decimals truncated toward zero
/// (`replay::fixed_point::q15_to_saved_fixed5`).
pub const PROPAGANDA_STABILITY_PENALTY: usize = 0x14c0;
/// Q15 being-bombed war-support penalty, consumed by displayed war support
/// and written under the being_bombed_support_penalty token (0x3c50).
pub const BEING_BOMBED_PENALTY: usize = 0x14c8;
/// Q15 heroes-killed penalty (`heroes_dying_war_support_penalty`, 0x3c51).
pub const HEROES_PENALTY: usize = 0x14d0;
/// Q15 convoys-raided penalty (`convoy_raiding_war_support_penalty`, 0x3c4f).
pub const CONVOYS_PENALTY: usize = 0x14d8;
/// Q15 propaganda war support penalty (`propaganda_war_support_penalty`, 0x4b47).
pub const PROPAGANDA_WAR_SUPPORT_PENALTY: usize = 0x14e0;
/// Fixed5 coastal-protection stability term written daily, added to displayed
/// stability and serialized as `coastal_protection_ratio`.
pub const COASTAL_PROTECTION_STABILITY: usize = 0x1600;
/// Air-statistics object used by displayed war support.
pub const AIR_STATISTICS: usize = 0xfc8;
/// Country accumulator children: data, capacity and count, visited by the
/// modifier-breakdown formatter.
pub const MODIFIER_CHILDREN: usize = ccountry::MODIFIER_ACCUMULATOR + 0x18;

/// Idea modifier accumulator, added with a built display label.
pub const IDEA_MODIFIER: usize = 0x68;
/// Country-leader modifier using character name as display text without a separate label.
pub const LEADER_MODIFIER: usize = 0x68;
/// Ruling-party ideology-group modifier.
pub const IDEOLOGY_GROUP_MODIFIER: usize = 0x480;
/// Advisor modifier.
pub const ADVISOR_MODIFIER: usize = 0x1a0;
/// Dynamic-modifier constant part with stored display text.
pub const DYNAMIC_CONSTANT: usize = 0xf8;
/// Dynamic-modifier scripted rows, stride 0xd8, with signed key and recorded
/// count. Evaluated values are stored per entry; nonzero values form one child.
pub const DYNAMIC_VALUE_ROWS: usize = 0x1b8;
pub const DYNAMIC_VALUE_ROW_STRIDE: usize = 0xd8;
/// Static-modifier database holding the bound static slots.
pub const STATIC_DATABASE_RVA: usize = 0x0331_7cc8;
/// Static-modifier script name stored as an MSVC string and hashed for lookup.
pub const STATIC_NAME: usize = 0x1a8;
/// Diplomacy word tested by mode-1 war predicates on the counterpart;
/// only values below one admit the war.
pub const DIPLOMACY_HOSTING: usize = 0x1a8;
/// Diplomacy word tested first by mode-0 war queries, used only in fingerprint.
pub const DIPLOMACY_WAR_COUNTER: usize = 0xa4;

/// Interception-ring layout: data pointer +8, signed size +0x20, signed
/// head +0x24 and wrapped byte +0x28. Each slot points to an i64 row pointer;
/// column 1 is summed with the stored saturation limit.
const RING: usize = 0x10;
const RING_DATA: usize = 0x08;
const RING_SIZE: usize = 0x20;
const RING_COLUMN: usize = 1;
const RING_SATURATION: i64 = 0x53e2_d620_fb40;

/// Define globals in `CountryStabilityInputRow` field order. Loaded values
/// use fixed5 except interception war-support scale, which uses Q15.
pub const DEFINE_RVAS: [usize; DEFINES] = [
    0x0331_8b60, // BASE_STABILITY_WAR_FACTOR
    0x0331_8c08, // BASE_STABILITY_PARTY_POPULARITY_FACTOR
    0x0331_a7e8, // MIN_STABILITY
    0x0331_a750, // MAX_STABILITY
    0x0331_8f10, // WAR_SUPPORT_OFFNSIVE_WAR
    0x0331_8f98, // WAR_SUPPORT_DEFENSIVE_WAR
    0x0331_9020, // WAR_SUPPORT_TENSION_IMPACT
    0x0331_a918, // MIN_WAR_SUPPORT
    0x0331_a880, // MAX_WAR_SUPPORT
    0x0331_c360, // INTERCEPTION_WAR_SUPPORT_SCALE, Q15
    0x0331_c410, // INTERCEPTION_BOMBING_WAR_SUPPORT_IMPACT
    0x0331_c788, // BASE_MAX_COMMAND_POWER
];
pub const DEFINES: usize = 12;

/// Byte offsets of the bound static slots in the static database, in the
/// order that the modifier rebuild adds them.
#[derive(Clone, Copy)]
struct StaticSlot {
    offset: usize,
    scaled: bool,
    /// Position among the replayed sources: after the advisors (1), after the
    /// fleet statics (2) or after the dynamic modifiers (3).
    phase: u8,
}
const STATIC_SLOTS: [StaticSlot; 11] = [
    // Sent-attache static followed by lacking-consumer-goods static.
    StaticSlot {
        offset: 0x90,
        scaled: false,
        phase: 1,
    },
    StaticSlot {
        offset: 0x60,
        scaled: false,
        phase: 1,
    },
    // Fleet-pride static, peace/war static, then temporary sunk-pride static.
    StaticSlot {
        offset: 0xb8,
        scaled: false,
        phase: 2,
    },
    StaticSlot {
        offset: 0x390,
        scaled: false,
        phase: 2,
    },
    StaticSlot {
        offset: 0x398,
        scaled: false,
        phase: 2,
    },
    StaticSlot {
        offset: 0xc0,
        scaled: false,
        phase: 2,
    },
    // Scaled war-support good/bad statics, wartime war-support static, then
    // scaled stability good/bad statics.
    StaticSlot {
        offset: 0x78,
        scaled: true,
        phase: 3,
    },
    StaticSlot {
        offset: 0x80,
        scaled: true,
        phase: 3,
    },
    StaticSlot {
        offset: 0x88,
        scaled: true,
        phase: 3,
    },
    StaticSlot {
        offset: 0x68,
        scaled: true,
        phase: 3,
    },
    StaticSlot {
        offset: 0x70,
        scaled: true,
        phase: 3,
    },
];

/// Entries read per guarded copy.
const ENTRY_BATCH: usize = 32;

/// The source kinds a line can be paired with; replay maps them to its tags.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Unpaired,
    Idea,
    CountryLeader,
    Ideology,
    Advisor,
    Static,
    StaticScaled,
    DynamicModifier,
    DynamicModifierValues,
}

/// Byte range of a copied string in the walk's arena.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

/// One country's formula inputs; see the replay row for the meaning of each.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InputRow {
    pub country: u32,
    pub stability_raw: i64,
    pub war_support_raw: i64,
    pub at_war: bool,
    pub offensive_war: bool,
    pub defensive_war: bool,
    pub ruling_party_popularity_raw: i64,
    pub propaganda_stability_q15: i64,
    pub coastal_protection_stability_raw: i64,
    pub bombing_q15: i64,
    pub heroes_q15: i64,
    pub convoys_q15: i64,
    pub propaganda_war_support_q15: i64,
    pub interception_sum_raw: i64,
    /// The getter value of each of the first [`TOTALS`] tracked keys; None
    /// when the key is unregistered.
    pub totals: [Option<i64>; TOTALS],
    pub children: u32,
}

/// One (child, tracked key) with a nonzero value.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LineRow {
    pub country: u32,
    pub line: u32,
    pub key: u32,
    pub kind: Kind,
    pub identity: Span,
    pub identity_id: u32,
    pub secondary: i64,
    pub value: i64,
    /// The child's display text; only for [`Kind::Unpaired`].
    pub text: Span,
}

/// Tracked key indices resolved by name, and the definition table header they
/// were resolved against: they stay valid while that header is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keys {
    pub keys: [Option<u32>; TRACKED],
    pub table: Array,
}

impl Keys {
    pub fn resolve(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let (keys, table) = modifier_definition::keys_by_names_at(memory, base, &KEY_NAMES)?;
        Ok(Self { keys, table })
    }

    /// Whether the definition table still has the header these keys came from.
    /// Whether the resolved keys still hold: the modifier table is unchanged. A
    /// failed read is an error, never a reason to resolve again.
    pub fn current(&self, memory: &impl Memory, base: usize) -> Result<bool, &'static str> {
        modifier_definition::table(memory, base).map(|table| table == self.table)
    }

    /// Registered keys ascending, without repeats, for one accumulator census.
    fn sorted(&self) -> ([u32; TRACKED], usize) {
        Self::ascending(self.keys.iter().flatten())
    }

    /// [`Self::sorted`] for the keys whose totals are recorded ([`TOTALS`]).
    fn sorted_totals(&self) -> ([u32; TRACKED], usize) {
        Self::ascending(self.keys[..TOTALS].iter().flatten())
    }

    fn ascending<'k>(keys: impl Iterator<Item = &'k u32>) -> ([u32; TRACKED], usize) {
        let mut sorted = [0; TRACKED];
        let mut len = 0;
        for key in keys {
            if !sorted[..len].contains(key) {
                sorted[len] = *key;
                len += 1;
            }
        }
        sorted[..len].sort_unstable();
        (sorted, len)
    }
}

/// Per-walk globals: defines, world tension, the active language, the bound
/// statics and the tracked keys' metadata.
#[derive(Debug)]
pub struct Context {
    pub keys: Keys,
    pub defines: [i64; DEFINES],
    pub world_tension_raw: i64,
    pub language: Vec<u8>,
    operands: [Option<modifier_definition::Operands>; TRACKED],
    statics: [usize; STATIC_SLOTS.len()],
}

impl Context {
    pub fn read(
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        keys: Keys,
    ) -> Result<Self, &'static str> {
        let mut defines = [0; DEFINES];
        for (value, rva) in defines.iter_mut().zip(DEFINE_RVAS) {
            *value = read::<i64>(memory, at(base, rva)?).ok_or("stability define unreadable")?;
        }
        let mut manager = [cworld_threat::ManagerRow::default()];
        let world = cworld_threat::Reader::new(memory, base, game_state)?;
        if world.manager(&mut manager) != (Outcome::Rows { len: 1 }) {
            return Err("world tension unreadable");
        }
        let mut operands = [None; TRACKED];
        for (operand, key) in operands.iter_mut().zip(keys.keys) {
            if let Some(key) = key {
                *operand = Some(modifier_definition::operands(memory, base, key)?);
            }
        }
        let database = pointer(memory, at(base, STATIC_DATABASE_RVA)?)?;
        let static_vtable = at(base, cstatic_modifier::VTABLE_RVA)? as u64;
        let mut statics = [0; STATIC_SLOTS.len()];
        for (value, slot) in statics.iter_mut().zip(STATIC_SLOTS) {
            let address = read::<u64>(memory, at(database, slot.offset)?)
                .ok_or("static modifier slot unreadable")?;
            let address = usize::try_from(address).map_err(|_| "static modifier slot overflow")?;
            if address != 0 && read::<u64>(memory, address) != Some(static_vtable) {
                return Err("static modifier slot vtable mismatch");
            }
            *value = address;
        }
        Ok(Self {
            keys,
            defines,
            world_tension_raw: manager[0].total_raw,
            language: language(memory, base)?,
            operands,
            statics,
        })
    }
}

/// Active-language singleton: its first word points to the language string.
/// A null root or string supplies the empty result.
fn language(memory: &impl Memory, base: usize) -> Result<Vec<u8>, &'static str> {
    let root = read::<u64>(memory, at(base, localization_root_layout::SINGLETON_RVA)?)
        .ok_or("language root unreadable")?;
    if root == 0 {
        return Ok(Vec::new());
    }
    let root = usize::try_from(root).map_err(|_| "language root overflow")?;
    let string = read::<u64>(memory, at(root, localization_root_layout::LANGUAGE_STRING)?)
        .ok_or("language string pointer unreadable")?;
    if string == 0 {
        return Ok(Vec::new());
    }
    let string = usize::try_from(string).map_err(|_| "language string overflow")?;
    let descriptor = Descriptor::read(memory, string)?;
    let mut text = Vec::new();
    descriptor.copy_buffer(memory, &mut Buffer::Growing(&mut text), 0)?;
    Ok(text)
}

/// Engine layout for one digest of nonzero `(key, value)` entries in order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Signature {
    count: u32,
    digest: u64,
}

impl Signature {
    fn add(&mut self, key: u32, value: i64) {
        self.count += 1;
        for byte in key.to_le_bytes().into_iter().chain(value.to_le_bytes()) {
            self.digest = (self.digest ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn new() -> Self {
        Self {
            count: 0,
            digest: 0xcbf2_9ce4_8422_2325,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Child {
    pointer: usize,
    entries: Array,
    signature: Signature,
    display: (usize, u64),
    tracked: [i64; TRACKED],
    lines: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Content {
    #[default]
    None,
    Entries(Array),
    Values {
        start: usize,
        len: usize,
    },
}

#[derive(Debug, Default, Clone, Copy)]
struct Candidate {
    kind: Kind,
    content: Content,
    signature: Signature,
    /// The source's display string and its (length, digest), compared with
    /// the child's when the child copies it.
    display: Option<(usize, usize, u64)>,
    /// A scaled static: paired by display text and the same set of nonzero keys
    /// (its values are scaled, so they are not compared).
    scaled: bool,
    /// A nonzero entry has a tracked key. Only such candidates can equal a
    /// child with lines, so no other one takes part in pairing.
    tracked: bool,
    identity: Option<usize>,
    identity_id: u32,
    secondary: i64,
}

/// Preallocated per-walk storage; allocated off the game thread.
pub struct Scratch {
    children: Vec<Child>,
    candidates: Vec<Candidate>,
    values: Vec<(u32, i64)>,
    forward: Vec<u32>,
    backward: Vec<u32>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub inputs: usize,
    pub lines: usize,
    pub bytes: usize,
    pub children: usize,
    pub candidates: usize,
    pub values: usize,
}

const NONE: u32 = u32::MAX;

impl Scratch {
    pub fn with_demand(demand: Demand) -> std::io::Result<Self> {
        Ok(Self {
            children: crate::allocation::default_vec(demand.children)?,
            candidates: crate::allocation::default_vec(demand.candidates)?,
            values: crate::allocation::default_vec(demand.values)?,
            forward: crate::allocation::filled(demand.children, NONE)?,
            backward: crate::allocation::filled(demand.children, NONE)?,
        })
    }

    #[cfg(test)]
    pub fn new() -> Self {
        Self::vacant()
    }

    /// An allocation-free placeholder; worker walks grow from original headers.
    pub fn vacant() -> Self {
        Self {
            children: Vec::new(),
            candidates: Vec::new(),
            values: Vec::new(),
            forward: Vec::new(),
            backward: Vec::new(),
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.children.capacity() * size_of::<Child>()
            + self.candidates.capacity() * size_of::<Candidate>()
            + self.values.capacity() * size_of::<(u32, i64)>()
            + (self.forward.capacity() + self.backward.capacity()) * size_of::<u32>()
    }
}

impl Default for Scratch {
    fn default() -> Self {
        Self::vacant()
    }
}

/// One country's walk into caller-owned rows and bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountryWalk {
    /// Lines written, or the failure; a country whose lines do not fit writes
    /// none of them and counts all as unread.
    pub lines: Outcome,
    pub bytes_len: usize,
    /// Bytes a retry needs for this country's lines.
    pub required_bytes: usize,
}

/// Reads one authenticated country. On error nothing is written.
#[allow(clippy::too_many_arguments)]
pub fn walk_country<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: ccountry::Country,
    context: &Context,
    scratch: &mut Scratch,
    input: &mut InputRow,
    lines: &mut [LineRow],
    bytes: &mut [u8],
) -> Result<CountryWalk, &'static str> {
    walk_country_buffers(
        memory,
        base,
        countries,
        country,
        context,
        scratch,
        input,
        Buffer::Fixed(lines),
        Buffer::Fixed(bytes),
        0,
        0,
    )
}

/// Worker-only append. Child/source storage follows the original headers and
/// emitted rows/text grow before copying each native occurrence.
#[allow(clippy::too_many_arguments)]
pub fn walk_country_growing<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: ccountry::Country,
    context: &Context,
    scratch: &mut Scratch,
    input: &mut InputRow,
    lines: &mut Vec<LineRow>,
    bytes: &mut Vec<u8>,
    line_start: usize,
    byte_start: usize,
) -> Result<CountryWalk, &'static str> {
    walk_country_buffers(
        memory,
        base,
        countries,
        country,
        context,
        scratch,
        input,
        Buffer::Growing(lines),
        Buffer::Growing(bytes),
        line_start,
        byte_start,
    )
}

#[allow(clippy::too_many_arguments)]
fn walk_country_buffers<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: ccountry::Country,
    context: &Context,
    scratch: &mut Scratch,
    input: &mut InputRow,
    lines: Buffer<'_, LineRow>,
    bytes: Buffer<'_, u8>,
    line_start: usize,
    byte_start: usize,
) -> Result<CountryWalk, &'static str> {
    *input = inputs(memory, base, countries, country, context)?;
    let children = read_children(memory, base, country.pointer, context, scratch)?;
    if children != input.children as usize {
        return Err("country modifier tree changed during the walk");
    }
    let candidates = candidates(memory, base, country, context, scratch)?;
    pair(memory, scratch, children, candidates)?;
    emit(
        memory, country.id, context, scratch, children, lines, bytes, line_start, byte_start,
    )
}

/// Exact emitted line/text demand follows appended modifier clones in source
/// order, preserving acquisition pairing and tracked entries.
pub fn demand(memory: &impl Memory, base: usize, state: usize) -> Result<Demand, &'static str> {
    crate::allocation::advisory(|| demand_advisory(memory, base, state))
}

fn demand_advisory(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<Demand, &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let keys = Keys::resolve(memory, base)?;
    let context = Context::read(memory, base, state, keys)?;
    let mut scratch = Scratch::vacant();
    let mut rows = Vec::new();
    let mut bytes = Vec::new();
    let mut demand = Demand::default();
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let walk = walk_country_growing(
            memory,
            base,
            &countries,
            country,
            &context,
            &mut scratch,
            &mut InputRow::default(),
            &mut rows,
            &mut bytes,
            0,
            0,
        )?;
        let lines = match walk.lines {
            Outcome::Rows { len } => len,
            Outcome::Truncated { len, unread_rows } => len
                .checked_add(unread_rows as usize)
                .ok_or("stability sizing row count overflow")?,
            Outcome::Unavailable { reason } => return Err(reason),
        };
        demand.inputs = demand
            .inputs
            .checked_add(1)
            .ok_or("stability sizing country count overflow")?;
        demand.lines = demand
            .lines
            .checked_add(lines)
            .ok_or("stability sizing row count overflow")?;
        demand.bytes = demand
            .bytes
            .checked_add(walk.required_bytes)
            .ok_or("stability sizing bytes overflow")?;
    }
    demand.children = scratch.children.len();
    demand.candidates = scratch.candidates.len();
    demand.values = scratch.values.len();
    Ok(demand)
}

fn inputs<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: ccountry::Country,
    context: &Context,
) -> Result<InputRow, &'static str> {
    let c = country.pointer;
    let mut bases = [0_u8; 16];
    let mut penalties = [0_u8; 40];
    let mut coastal = [0_u8; 8];
    let mut children = [0_u8; 16];
    if memory.copy_scattered(
        [
            at(c, ccountry::STABILITY)?,
            at(c, PROPAGANDA_STABILITY_PENALTY)?,
            at(c, COASTAL_PROTECTION_STABILITY)?,
            at(c, MODIFIER_CHILDREN)?,
        ],
        &mut [&mut bases, &mut penalties, &mut coastal, &mut children],
    ) != 4
    {
        return Err("country stability fields unreadable");
    }
    let children = Array::from_header(children, 8).ok_or("country modifier children invalid")?;
    let (at_war, offensive_war, defensive_war) = war_flags(memory, base, countries, country)?;
    let mut totals = [None; TOTALS];
    let (sorted, len) = context.keys.sorted_totals();
    let mut values = [modifier_accumulator::Value::default(); TRACKED];
    modifier_accumulator::lookup_many_at(
        memory,
        at(c, ccountry::MODIFIER_ACCUMULATOR)?,
        &sorted[..len],
        &mut values[..len],
    )?;
    for ((total, key), operands) in totals
        .iter_mut()
        .zip(context.keys.keys)
        .zip(context.operands)
    {
        if let (Some(key), Some(operands)) = (key, operands) {
            let value = values[sorted[..len].iter().position(|k| *k == key).unwrap()];
            *total = Some(if value.present {
                operands.effective(value.raw)
            } else {
                0
            });
        }
    }
    Ok(InputRow {
        country: country.id,
        stability_raw: i64_at(&bases, 0),
        war_support_raw: i64_at(&bases, 8),
        at_war,
        offensive_war,
        defensive_war,
        ruling_party_popularity_raw: popularity(memory, base, c)?,
        propaganda_stability_q15: i64_at(&penalties, 0),
        bombing_q15: i64_at(&penalties, 8),
        heroes_q15: i64_at(&penalties, 16),
        convoys_q15: i64_at(&penalties, 24),
        propaganda_war_support_q15: i64_at(&penalties, 32),
        coastal_protection_stability_raw: i64_at(&coastal, 0),
        interception_sum_raw: interception(memory, c)?,
        totals,
        children: children.count,
    })
}

/// A change fingerprint for the sliced family: the bases, stored penalty and
/// coastal terms, ruling party popularity, the war counter word, the tree's
/// child header and the tracked keys' raw root entries. It only adds reads.
pub fn fingerprint_country(
    memory: &impl Memory,
    base: usize,
    country: usize,
    keys: &Keys,
) -> Result<u64, &'static str> {
    let mut bases = [0_u8; 16];
    let mut penalties = [0_u8; 40];
    let mut coastal = [0_u8; 8];
    let mut children = [0_u8; 16];
    let mut diplomacy = [0_u8; 8];
    if memory.copy_scattered(
        [
            at(country, ccountry::STABILITY)?,
            at(country, PROPAGANDA_STABILITY_PENALTY)?,
            at(country, COASTAL_PROTECTION_STABILITY)?,
            at(country, MODIFIER_CHILDREN)?,
            at(country, ccountry::DIPLOMACY_STATUS)?,
        ],
        &mut [
            &mut bases,
            &mut penalties,
            &mut coastal,
            &mut children,
            &mut diplomacy,
        ],
    ) != 5
    {
        return Err("country stability fingerprint unreadable");
    }
    let diplomacy = pointer_from(&diplomacy)?;
    let war = read::<i32>(memory, at(diplomacy, DIPLOMACY_WAR_COUNTER)?)
        .ok_or("country war counter unreadable")?;
    let (sorted, len) = keys.sorted();
    let mut values = [modifier_accumulator::Value::default(); TRACKED];
    modifier_accumulator::lookup_many_at(
        memory,
        at(country, ccountry::MODIFIER_ACCUMULATOR)?,
        &sorted[..len],
        &mut values[..len],
    )?;
    let mut digest = Signature::new();
    for chunk in [&bases[..], &penalties, &coastal, &children[8..]] {
        for word in chunk.chunks_exact(8) {
            digest.add(0, i64_at(word, 0));
        }
    }
    digest.add(1, i64::from(war));
    digest.add(2, popularity(memory, base, country)?);
    for (key, value) in sorted[..len].iter().zip(&values[..len]) {
        digest.add(*key, if value.present { value.raw } else { i64::MIN });
    }
    sources(memory, base, country, &mut digest)?;
    Ok(digest.digest)
}

/// The identities of the replayed sources: a swap between two sources with the
/// same tracked values (a law or spirit replaced by one with other untracked
/// effects) keeps the totals and the tree size, so the objects themselves are
/// covered: the active ideas, the ruling party and its first leader, the
/// appointed advisors and each dynamic modifier entry's definition, scope and
/// enabled state (an entry replaced in place keeps the array header).
fn sources(
    memory: &impl Memory,
    base: usize,
    country: usize,
    digest: &mut Signature,
) -> Result<(), &'static str> {
    let mut object = |tag: u32, address: usize| digest.add(tag, address as i64);
    let status = pointer(memory, at(country, ccountry::POLITICAL_STATUS)?)?;
    let ideas = Array::read(memory, at(status, cpolitical_status::IDEAS)?, 8)
        .ok_or("active idea array invalid")?;
    object(3, ideas.count as usize);
    pointer_pages(memory, ideas, |idea| {
        object(4, idea);
        Ok(())
    })?;
    let party = pointer(memory, at(status, cpolitical_status::RULING_PARTY)?)?;
    object(5, party);
    let leaders = Array::read(memory, at(party, cpolitical_party::COUNTRY_LEADERS)?, 8)
        .ok_or("ruling party leaders invalid")?;
    if leaders.count != 0 {
        object(6, pointer(memory, leaders.data)?);
    }
    let characters = read::<u64>(memory, at(country, ccountry::CHARACTERS)?)
        .ok_or("country characters pointer unreadable")?;
    if characters != 0 {
        let characters = usize::try_from(characters).map_err(|_| "country characters overflow")?;
        let advisors = Array::read(
            memory,
            at(characters, ccountry_characters::APPOINTED_ADVISORS)?,
            8,
        )
        .ok_or("appointed advisor array invalid")?;
        object(7, advisors.count as usize);
        pointer_pages(memory, advisors, |advisor| {
            object(8, advisor);
            Ok(())
        })?;
    }
    let entries = Array::read(
        memory,
        at(
            at(country, ccountry::DYNAMIC_MODIFIERS)?,
            cdynamic_modifier_container::ENTRIES,
        )?,
        dynamic_modifier_entry::STRIDE,
    )
    .ok_or("dynamic modifier entries invalid")?;
    object(9, entries.count as usize);
    for index in 0..entries.count {
        let entry = entries
            .element(index, dynamic_modifier_entry::STRIDE)
            .ok_or("dynamic modifier entry overflow")?;
        let header = dynamic_modifier_entry::header(memory, base, entry)?;
        digest.add(10, header.definition as i64);
        digest.add(
            11,
            (i64::from(header.scope_country) << 32) | i64::from(header.scope_state as u32),
        );
        digest.add(12, i64::from(header.enabled));
    }
    Ok(())
}

/// Authenticated ruling-party popularity selected through country political status.
fn popularity(memory: &impl Memory, base: usize, country: usize) -> Result<i64, &'static str> {
    let status = pointer(memory, at(country, ccountry::POLITICAL_STATUS)?)?;
    let head = read::<[u8; 8]>(memory, status).ok_or("political status unreadable")?;
    if u64::from_le_bytes(head) != at(base, cpolitical_status::VTABLE_RVA)? as u64 {
        return Err("political status vtable mismatch");
    }
    let party = pointer(memory, at(status, cpolitical_status::RULING_PARTY)?)?;
    if read::<u64>(memory, party) != Some(at(base, cpolitical_party::VTABLE_RVA)? as u64) {
        return Err("ruling party vtable mismatch");
    }
    read::<i64>(memory, at(party, cpolitical_party::POPULARITY)?)
        .ok_or("ruling party popularity unreadable")
}

/// Owned war-relation predicates retain relation type, attacker/defender roles
/// and `ccountry_tag::same_country` identity. Mode 1 additionally requires
/// the second country's diplomacy word to be below one.
fn war_flags<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: ccountry::Country,
) -> Result<(bool, bool, bool), &'static str> {
    let diplomacy = pointer(memory, at(country.pointer, ccountry::DIPLOMACY_STATUS)?)?;
    let mut head = [0_u8; 8];
    let mut owned = [0_u8; 16];
    let mut owner = [0_u8; 8];
    if memory.copy_scattered(
        [
            diplomacy,
            at(diplomacy, cdiplomacy_status::OWNED_RELATIONS)?,
            at(diplomacy, cdiplomacy_status::COUNTRY)?,
        ],
        &mut [&mut head, &mut owned, &mut owner],
    ) != 3
    {
        return Err("country diplomacy unreadable");
    }
    if u64::from_le_bytes(head) != at(base, cdiplomacy_status::VTABLE_RVA)? as u64 {
        return Err("country diplomacy vtable mismatch");
    }
    if pointer_from(&owner)? != country.pointer {
        return Err("country diplomacy owner is another country");
    }
    let owned = Array::from_header(owned, 8).ok_or("owned relation array invalid")?;
    let war_vtable = at(base, cwar_relation::VTABLE_RVA)? as u64;
    let same = |raw: u32| -> Result<bool, &'static str> {
        Ok(raw == country.id
            || (raw != 0
                && country.id != 0
                && countries.link(raw)? == countries.link(country.id)?))
    };
    let (mut at_war, mut offensive, mut defensive) = (false, false, false);
    pointer_pages(memory, owned, |relation| {
        let header = read::<[u8; 0x18]>(memory, relation).ok_or("relation unreadable")?;
        if u32::from_le_bytes(
            header[crelation::TYPE_ID..crelation::TYPE_ID + 4]
                .try_into()
                .unwrap(),
        ) != cwar_relation::TYPE
        {
            return Ok(());
        }
        if u64::from_le_bytes(header[..8].try_into().unwrap()) != war_vtable {
            return Err("war relation vtable mismatch");
        }
        let first = u32::from_le_bytes(header[0x10..0x14].try_into().unwrap());
        let second = u32::from_le_bytes(header[0x14..0x18].try_into().unwrap());
        let instigator = read::<u8>(memory, at(relation, cwar_relation::FIRST_WAS_INSTIGATOR)?)
            .ok_or("war relation instigator unreadable")?;
        let (attacker, defender) = if instigator != 0 {
            (first, second)
        } else {
            (second, first)
        };
        offensive |= same(attacker)?;
        defensive |= same(defender)?;
        if !at_war {
            let counterpart = countries.resolve(second)?;
            let other = pointer(memory, at(counterpart.pointer, ccountry::DIPLOMACY_STATUS)?)?;
            let hosting = read::<i32>(memory, at(other, DIPLOMACY_HOSTING)?)
                .ok_or("war counterpart diplomacy unreadable")?;
            at_war = hosting < 1;
        }
        Ok(())
    })?;
    Ok((at_war, offensive, defensive))
}

/// Interception-ring sum for the country's air-statistics object, read passively.
fn interception(memory: &impl Memory, country: usize) -> Result<i64, &'static str> {
    let stats = pointer(memory, at(country, AIR_STATISTICS)?)?;
    let ring = pointer(memory, at(stats, RING)?)?;
    let header = read::<[u8; 0x29]>(memory, ring).ok_or("interception ring unreadable")?;
    let size = i32::from_le_bytes(header[RING_SIZE..RING_SIZE + 4].try_into().unwrap());
    let head = i32::from_le_bytes(header[RING_SIZE + 4..RING_SIZE + 8].try_into().unwrap());
    let wrapped = header[RING_SIZE + 8] != 0;
    let count = (if wrapped { size } else { head }).min(size);
    if count <= 0 {
        return Ok(0);
    }
    let data = pointer_from(&header[RING_DATA..RING_DATA + 8])?;
    let mut sum = 0_i64;
    for index in 0..count {
        let slot = if wrapped {
            head.wrapping_add(index) % size
        } else {
            index
        };
        let slot = usize::try_from(slot).map_err(|_| "interception ring slot negative")?;
        let row = pointer(memory, at(data, slot * 8)?)?;
        let values = pointer(memory, row)?;
        let value = read::<i64>(memory, at(values, RING_COLUMN * 8)?)
            .ok_or("interception value unreadable")?;
        sum = sum
            .wrapping_add(value)
            .clamp(-RING_SATURATION, RING_SATURATION);
    }
    Ok(sum)
}

/// Copies every child's header, display and entries: nonzero-entry
/// signature, display digest and tracked values.
fn read_children(
    memory: &impl Memory,
    base: usize,
    country: usize,
    context: &Context,
    scratch: &mut Scratch,
) -> Result<usize, &'static str> {
    let children = Array::read(memory, at(country, MODIFIER_CHILDREN)?, 8)
        .ok_or("country modifier children unreadable")?;
    let count = children.count as usize;
    if scratch.children.len() < count {
        crate::allocation::resize_default(&mut scratch.children, count)
            .map_err(|_| "stability child storage allocation failed")?;
        crate::allocation::resize(&mut scratch.forward, count, NONE)
            .map_err(|_| "stability pairing storage allocation failed")?;
        crate::allocation::resize(&mut scratch.backward, count, NONE)
            .map_err(|_| "stability pairing storage allocation failed")?;
    }
    let vtable = at(base, cmodifier::VTABLE_RVA)? as u64;
    let mut index = 0;
    pointer_pages(memory, children, |child| {
        let mut head = [0_u8; 8];
        let mut accumulator = [0_u8; 16];
        let mut display = [0_u8; 32];
        if memory.copy_scattered(
            [
                child,
                at(child, cmodifier::ACCUMULATOR)?,
                at(child, cmodifier::DISPLAY_NAME)?,
            ],
            &mut [&mut head, &mut accumulator, &mut display],
        ) != 3
        {
            return Err("country modifier child unreadable");
        }
        if u64::from_le_bytes(head) != vtable {
            return Err("country modifier child vtable mismatch");
        }
        let entries = Array::from_header(accumulator, modifier_entry::STRIDE)
            .ok_or("country modifier child entries invalid")?;
        let mut tracked = [0; TRACKED];
        let signature = signature(memory, entries, |key, value| {
            for (slot, tracked_key) in tracked.iter_mut().zip(context.keys.keys) {
                if tracked_key == Some(key) {
                    *slot = value;
                }
            }
        })?;
        let address = at(child, cmodifier::DISPLAY_NAME)?;
        let display = display_digest(memory, address, display)?;
        scratch.children[index] = Child {
            pointer: child,
            entries,
            signature,
            display: (address, display.1),
            lines: tracked.iter().filter(|value| **value != 0).count() as u32,
            tracked,
        };
        index += 1;
        Ok(())
    })?;
    Ok(index)
}

/// A validated string's length and digest, streamed without a text-size cap.
fn display_digest(
    memory: &impl Memory,
    address: usize,
    header: [u8; 32],
) -> Result<(usize, u64), &'static str> {
    let descriptor = Descriptor::from_header(memory, address, header)?;
    let mut bytes = [0_u8; 1024];
    let mut digest = 0xcbf2_9ce4_8422_2325_u64;
    for start in (0..descriptor.len).step_by(bytes.len()) {
        let count = (descriptor.len - start).min(bytes.len());
        descriptor.copy_chunk(memory, start, &mut bytes[..count])?;
        for value in &bytes[..count] {
            for byte in u32::from(*value)
                .to_le_bytes()
                .into_iter()
                .chain(0_i64.to_le_bytes())
            {
                digest = (digest ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    Ok((descriptor.len, digest))
}

/// Digests only reject; matching digests still require exact payload equality.
fn same_text(
    memory: &impl Memory,
    left: usize,
    right: usize,
    expected_len: usize,
) -> Result<bool, &'static str> {
    let left = Descriptor::read(memory, left)?;
    let right = Descriptor::read(memory, right)?;
    if left.len != expected_len || right.len != expected_len {
        return Ok(false);
    }
    let mut a = [0_u8; 1024];
    let mut b = [0_u8; 1024];
    for start in (0..expected_len).step_by(a.len()) {
        let count = (expected_len - start).min(a.len());
        left.copy_chunk(memory, start, &mut a[..count])?;
        right.copy_chunk(memory, start, &mut b[..count])?;
        if a[..count] != b[..count] {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Walks the nonzero entries of a sorted accumulator array, validating the
/// whole array's key order.
fn nonzero(
    memory: &impl Memory,
    entries: Array,
    mut visit: impl FnMut(u32, i64) -> Result<bool, &'static str>,
) -> Result<(), &'static str> {
    let mut bytes = [0_u8; ENTRY_BATCH * modifier_entry::STRIDE];
    let mut previous = 0;
    let mut first = 0;
    while first < entries.count {
        let count = ((entries.count - first) as usize).min(ENTRY_BATCH);
        let address = entries
            .element(first, modifier_entry::STRIDE)
            .ok_or("modifier entry address overflow")?;
        let chunk = &mut bytes[..count * modifier_entry::STRIDE];
        if !read_slice(memory, address, chunk) {
            return Err("modifier entries unreadable");
        }
        for entry in chunk.chunks_exact(modifier_entry::STRIDE) {
            let key = u32::from_le_bytes(entry[..4].try_into().unwrap());
            if key <= previous {
                return Err("modifier entry keys not positive and increasing");
            }
            previous = key;
            let value = i64_at(entry, modifier_entry::VALUE);
            if value != 0 && !visit(key, value)? {
                return Ok(());
            }
        }
        first += count as u32;
    }
    Ok(())
}

fn signature(
    memory: &impl Memory,
    entries: Array,
    mut each: impl FnMut(u32, i64),
) -> Result<Signature, &'static str> {
    let mut signature = Signature::new();
    nonzero(memory, entries, |key, value| {
        signature.add(key, value);
        each(key, value);
        Ok(true)
    })?;
    Ok(signature)
}

/// The replayed sources in modifier-rebuild order.
fn candidates(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
    context: &Context,
    scratch: &mut Scratch,
) -> Result<usize, &'static str> {
    let mut list = Builder {
        candidates: &mut scratch.candidates,
        values: &mut scratch.values,
        keys: context.keys.keys,
        len: 0,
        values_len: 0,
    };
    let c = country.pointer;
    // 1-3: ideas, the ruling party's first country leader, its ideology group.
    let status = pointer(memory, at(c, ccountry::POLITICAL_STATUS)?)?;
    let ideas = Array::read(memory, at(status, cpolitical_status::IDEAS)?, 8)
        .ok_or("active idea array invalid")?;
    let primary = at(base, cidea::VTABLE_RVA)? as u64;
    let secondary = at(base, cidea::SECONDARY_VTABLE_RVA)? as u64;
    pointer_pages(memory, ideas, |idea| {
        let vtable = read::<u64>(memory, idea).ok_or("active idea vtable unreadable")?;
        let object = if vtable == secondary {
            idea.checked_sub(cidea::SECONDARY_BASE)
                .ok_or("active idea primary underflow")?
        } else if vtable == primary {
            idea
        } else {
            return Err("active idea vtable mismatch");
        };
        if read::<u64>(memory, object) != Some(primary) {
            return Err("active idea primary vtable mismatch");
        }
        // Add the idea's own modifier pointer with a built display label.
        list.source(memory, at(idea, IDEA_MODIFIER)?, Kind::Idea, false)?
            .identity(at(object, cidea::TOKEN)?, 0, 0);
        Ok(())
    })?;
    let party = pointer(memory, at(status, cpolitical_status::RULING_PARTY)?)?;
    let mut leaders = [0_u8; 16];
    let mut group = [0_u8; 8];
    if memory.copy_scattered(
        [
            at(party, cpolitical_party::COUNTRY_LEADERS)?,
            at(party, cpolitical_party::IDEOLOGY_GROUP)?,
        ],
        &mut [&mut leaders, &mut group],
    ) != 2
    {
        return Err("ruling party unreadable");
    }
    let leaders = Array::from_header(leaders, 8).ok_or("ruling party leaders invalid")?;
    if leaders.count != 0 {
        // Select the first element only when count is nonzero.
        let leader = pointer(memory, leaders.data)?;
        let mut head = [0_u8; 16];
        if !read_slice(memory, leader, &mut head)
            || u64::from_le_bytes(head[..8].try_into().unwrap())
                != at(base, ccountry_leader::VTABLE_RVA)? as u64
        {
            return Err("country leader unreadable or vtable mismatch");
        }
        let character = character_id(memory, base, pointer_from(&head[8..])?)?;
        list.source(
            memory,
            at(leader, LEADER_MODIFIER)?,
            Kind::CountryLeader,
            true,
        )?
        .id(character, 0);
    }
    let group = pointer_from(&group)?;
    if read::<u64>(memory, group) != Some(at(base, cideology_group::VTABLE_RVA)? as u64) {
        return Err("ruling ideology group vtable mismatch");
    }
    list.source(
        memory,
        at(group, IDEOLOGY_GROUP_MODIFIER)?,
        Kind::Ideology,
        true,
    )?
    .identity(at(group, cideology_group::TOKEN)?, 0, 0);
    // 8: appointed advisors, after faction, difficulty and balance-of-power
    // sources this reader does not replay.
    let object = read::<u64>(memory, at(c, ccountry::CHARACTERS)?)
        .ok_or("country characters pointer unreadable")?;
    if object != 0 {
        let object = usize::try_from(object).map_err(|_| "country characters overflow")?;
        let head = read::<[u8; 16]>(memory, object).ok_or("country characters unreadable")?;
        if u64::from_le_bytes(head[..8].try_into().unwrap())
            != at(base, ccountry_characters::VTABLE_RVA)? as u64
            || pointer_from(&head[8..])? != c
        {
            return Err("country characters vtable or owner mismatch");
        }
        let advisors = Array::read(
            memory,
            at(object, ccountry_characters::APPOINTED_ADVISORS)?,
            8,
        )
        .ok_or("appointed advisor array invalid")?;
        let vtable = at(base, cadvisor::VTABLE_RVA)? as u64;
        pointer_pages(memory, advisors, |advisor| {
            let mut head = [0_u8; 24];
            if !read_slice(memory, advisor, &mut head)
                || u64::from_le_bytes(head[..8].try_into().unwrap()) != vtable
            {
                return Err("appointed advisor unreadable or vtable mismatch");
            }
            let character =
                character_id(memory, base, pointer_from(&head[cadvisor::CHARACTER..24])?)?;
            list.source(memory, at(advisor, ADVISOR_MODIFIER)?, Kind::Advisor, true)?
                .identity(at(advisor, cadvisor::IDEA_TOKEN)?, character, 0);
            Ok(())
        })?;
    }
    // 14-15 and 22-24: named statics (their conditions are not replayed; a
    // static that was not added simply pairs with no child). An inactive
    // static could only take another source's child with its exact display
    // and entries. Unreplayed kinds label their children with a localised
    // decision or mission prefix during the modifier rebuild, so this needs
    // content localising a static exactly as such a label.
    for phase in [1, 2] {
        list.statics(memory, context, phase)?;
    }
    // 28: enabled dynamic modifiers, the constant part then the values.
    let container = at(c, ccountry::DYNAMIC_MODIFIERS)?;
    let entries = Array::read(
        memory,
        at(container, cdynamic_modifier_container::ENTRIES)?,
        dynamic_modifier_entry::STRIDE,
    )
    .ok_or("dynamic modifier entries invalid")?;
    for index in 0..entries.count {
        let entry = entries
            .element(index, dynamic_modifier_entry::STRIDE)
            .ok_or("dynamic modifier entry overflow")?;
        let header = dynamic_modifier_entry::header(memory, base, entry)?;
        if header.enabled == 0 {
            continue;
        }
        let definition = header.definition;
        cdynamic_modifier::token(memory, base, definition)?;
        let id = at(definition, cdynamic_modifier::ID)?;
        let scope = (header.scope_country, i64::from(header.scope_state));
        list.source(
            memory,
            at(definition, DYNAMIC_CONSTANT)?,
            Kind::DynamicModifier,
            true,
        )?
        .identity(id, scope.0, scope.1);
        list.values(memory, definition, header.values)?
            .identity(id, scope.0, scope.1);
    }
    // 29-31: the scaled war support and stability statics.
    list.statics(memory, context, 3)?;
    Ok(list.len)
}

struct Builder<'a> {
    candidates: &'a mut Vec<Candidate>,
    values: &'a mut Vec<(u32, i64)>,
    keys: [Option<u32>; TRACKED],
    len: usize,
    values_len: usize,
}

impl Builder<'_> {
    fn push(&mut self, candidate: Candidate) -> Result<&mut Candidate, &'static str> {
        if self.candidates.len() <= self.len {
            crate::allocation::resize_default(
                self.candidates,
                self.len
                    .checked_add(1)
                    .ok_or("stability source count overflow")?,
            )
            .map_err(|_| "stability source storage allocation failed")?;
        }
        let slot = self
            .candidates
            .get_mut(self.len)
            .ok_or("stability source storage invalid")?;
        *slot = candidate;
        self.len += 1;
        Ok(slot)
    }

    /// A CModifier source at `modifier`: its accumulator entries, and its
    /// display string when the child copies it (no label passed).
    fn source(
        &mut self,
        memory: &impl Memory,
        modifier: usize,
        kind: Kind,
        display: bool,
    ) -> Result<&mut Candidate, &'static str> {
        let entries = Array::read(
            memory,
            at(modifier, cmodifier::ACCUMULATOR)?,
            modifier_entry::STRIDE,
        )
        .ok_or("modifier source entries invalid")?;
        let display = if display {
            let address = at(modifier, cmodifier::DISPLAY_NAME)?;
            let header = read::<[u8; 32]>(memory, address).ok_or("source display unreadable")?;
            let (len, digest) = display_digest(memory, address, header)?;
            Some((address, len, digest))
        } else {
            None
        };
        let keys = self.keys;
        let mut tracked = false;
        let signature = signature(memory, entries, |key, _| {
            tracked |= keys.contains(&Some(key));
        })?;
        self.push(Candidate {
            kind,
            content: Content::Entries(entries),
            signature,
            display,
            tracked,
            ..Candidate::default()
        })
    }

    fn statics(
        &mut self,
        memory: &impl Memory,
        context: &Context,
        phase: u8,
    ) -> Result<(), &'static str> {
        for (slot, address) in STATIC_SLOTS.iter().zip(context.statics) {
            if slot.phase != phase || address == 0 {
                continue;
            }
            let kind = if slot.scaled {
                Kind::StaticScaled
            } else {
                Kind::Static
            };
            let candidate = self.source(memory, address, kind, true)?;
            candidate.scaled = slot.scaled;
            candidate.identity = Some(at(address, STATIC_NAME)?);
        }
        Ok(())
    }

    /// Dynamic-modifier evaluated values are summed per key in key order. Its
    /// child uses the localised definition ID, also stored as the constant part's
    /// display even when that part is empty. Require equal text so another source
    /// with equal values cannot take the child. A language change can leave the
    /// stored text and fresh child unequal; the line then stays unpaired.
    fn values(
        &mut self,
        memory: &impl Memory,
        definition: usize,
        values: Array,
    ) -> Result<&mut Candidate, &'static str> {
        let count = cdynamic_modifier::value_count(memory, definition)?;
        if count != values.count {
            return Err("dynamic modifier values do not match their definition");
        }
        let rows = pointer(memory, at(definition, DYNAMIC_VALUE_ROWS)?)
            .or_else(|error| if count == 0 { Ok(0) } else { Err(error) })?;
        let start = self.values_len;
        let required = start
            .checked_add(count as usize)
            .ok_or("dynamic modifier value count overflow")?;
        if self.values.len() < required {
            crate::allocation::resize_default(self.values, required)
                .map_err(|_| "stability value storage allocation failed")?;
        }
        for index in 0..count as usize {
            let value = read::<i64>(memory, at(values.data, index * 8)?)
                .ok_or("dynamic modifier value unreadable")?;
            if value == 0 {
                continue;
            }
            let key = read::<i32>(memory, at(rows, index * DYNAMIC_VALUE_ROW_STRIDE)?)
                .ok_or("dynamic modifier value key unreadable")?;
            let key = u32::try_from(key).map_err(|_| "dynamic modifier value key negative")?;
            let entries = &mut self.values[start..self.values_len];
            if let Some(entry) = entries.iter_mut().find(|entry| entry.0 == key) {
                entry.1 = entry.1.wrapping_add(value);
                continue;
            }
            *self
                .values
                .get_mut(self.values_len)
                .ok_or("dynamic modifier value storage invalid")? = (key, value);
            self.values_len += 1;
        }
        let entries = &mut self.values[start..self.values_len];
        entries.sort_unstable_by_key(|entry| entry.0);
        let mut signature = Signature::new();
        let mut tracked = false;
        for (key, value) in entries.iter().filter(|entry| entry.1 != 0) {
            signature.add(*key, *value);
            tracked |= self.keys.contains(&Some(*key));
        }
        let len = self.values_len - start;
        let address = at(at(definition, DYNAMIC_CONSTANT)?, cmodifier::DISPLAY_NAME)?;
        let header =
            read::<[u8; 32]>(memory, address).ok_or("dynamic modifier display unreadable")?;
        let (display_len, digest) = display_digest(memory, address, header)?;
        let display = Some((address, display_len, digest));
        self.push(Candidate {
            kind: Kind::DynamicModifierValues,
            content: Content::Values { start, len },
            signature,
            display,
            tracked,
            ..Candidate::default()
        })
    }
}

impl Candidate {
    fn identity(&mut self, address: usize, id: u32, secondary: i64) {
        self.identity = Some(address);
        self.identity_id = id;
        self.secondary = secondary;
    }
    fn id(&mut self, id: u32, secondary: i64) {
        self.identity_id = id;
        self.secondary = secondary;
    }
}

/// A CCharacter's id behind a role's character pointer, authenticated.
fn character_id(memory: &impl Memory, base: usize, character: usize) -> Result<u32, &'static str> {
    let identity = read::<[u8; 16]>(memory, character).ok_or("character unreadable")?;
    if u64::from_le_bytes(identity[..8].try_into().unwrap())
        != at(base, ccharacter::VTABLE_RVA)? as u64
        || u32::from_le_bytes(
            identity[ccharacter::TYPE..ccharacter::ID]
                .try_into()
                .unwrap(),
        ) != ccharacter::REFERENCE_TYPE
    {
        return Err("character identity mismatch");
    }
    Ok(u32::from_le_bytes(
        identity[ccharacter::ID..16].try_into().unwrap(),
    ))
}

/// Whether a child is the candidate's clone: equal nonzero entries (and equal
/// display text where the child copies it); a scaled static by display alone.
fn matches(
    memory: &impl Memory,
    values: &[(u32, i64)],
    child: &Child,
    candidate: &Candidate,
) -> Result<bool, &'static str> {
    if let Some((address, len, digest)) = candidate.display {
        let (child_address, child_digest) = child.display;
        if child_digest != digest {
            return Ok(false);
        }
        if !same_text(memory, child_address, address, len)? {
            return Ok(false);
        }
    }
    if !candidate.scaled && child.signature != candidate.signature {
        return Ok(false);
    }
    // Exact verification of every nonzero entry, in order.
    let mut expected = [(0_u32, 0_i64); 64];
    let mut equal = true;
    let mut seen = 0_usize;
    match candidate.content {
        Content::None => return Ok(false),
        Content::Values { start, len } => {
            let mut wanted = values[start..start + len].iter().filter(|e| e.1 != 0);
            nonzero(memory, child.entries, |key, value| {
                equal = wanted.next() == Some(&(key, value));
                Ok(equal)
            })?;
            equal &= wanted.next().is_none();
        }
        Content::Entries(source) => {
            // Compare in windows of the source's nonzero entries.
            let mut offset = 0;
            while equal {
                let mut filled = 0;
                let mut skipped = 0;
                nonzero(memory, source, |key, value| {
                    if skipped < offset {
                        skipped += 1;
                        return Ok(true);
                    }
                    expected[filled] = (key, value);
                    filled += 1;
                    Ok(filled < expected.len())
                })?;
                let mut position = 0;
                let mut index = 0;
                nonzero(memory, child.entries, |key, value| {
                    if position < offset {
                        position += 1;
                        return Ok(true);
                    }
                    if index == filled {
                        return Ok(false);
                    }
                    equal &= expected[index].0 == key
                        && (candidate.scaled || expected[index].1 == value);
                    index += 1;
                    position += 1;
                    Ok(equal)
                })?;
                if equal && index != filled {
                    equal = false;
                }
                seen += filled;
                offset += filled;
                if filled < expected.len() {
                    break;
                }
            }
            equal &= seen == child.signature.count as usize;
        }
    }
    Ok(equal)
}

/// Leftmost and rightmost order-preserving assignments; a child is paired
/// only when both choose the same source.
fn pair(
    memory: &impl Memory,
    scratch: &mut Scratch,
    children: usize,
    candidates: usize,
) -> Result<(), &'static str> {
    let Scratch {
        children: child_rows,
        candidates: sources,
        values,
        forward,
        backward,
    } = scratch;
    // Only children with lines and candidates with tracked keys participate.
    // Empty signatures could match each other and shift bounds across real
    // sources: tooltip-only sources can produce children, while ideas with no
    // modifier and dynamic modifiers with no nonzero value produce none.
    let takes_part = |source: &Candidate| source.tracked;
    let mut next = 0;
    for index in 0..children {
        forward[index] = NONE;
        if child_rows[index].lines == 0 {
            continue;
        }
        for (source, candidate) in sources.iter().enumerate().take(candidates).skip(next) {
            if takes_part(candidate) && matches(memory, values, &child_rows[index], candidate)? {
                forward[index] = source as u32;
                next = source + 1;
                break;
            }
        }
    }
    let mut limit = candidates;
    for index in (0..children).rev() {
        backward[index] = NONE;
        if child_rows[index].lines == 0 {
            continue;
        }
        for (source, candidate) in sources[..limit].iter().enumerate().rev() {
            if takes_part(candidate) && matches(memory, values, &child_rows[index], candidate)? {
                backward[index] = source as u32;
                limit = source;
                break;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit(
    memory: &impl Memory,
    country: u32,
    context: &Context,
    scratch: &Scratch,
    children: usize,
    mut lines: Buffer<'_, LineRow>,
    bytes: Buffer<'_, u8>,
    line_start: usize,
    byte_start: usize,
) -> Result<CountryWalk, &'static str> {
    let wanted = scratch.children[..children]
        .iter()
        .try_fold(0_u32, |total, child| {
            total
                .checked_add(child.lines)
                .ok_or("stability line count overflow")
        })?;
    let mut len = 0;
    let required_lines = line_start
        .checked_add(wanted as usize)
        .ok_or("stability line count overflow")?;
    let fits = lines.ensure(required_lines)?;
    let mut arena = Arena {
        bytes,
        used: byte_start,
        required: byte_start,
        fits,
    };
    for (line, child) in scratch.children[..children].iter().enumerate() {
        if child.lines == 0 {
            continue;
        }
        let paired = (scratch.forward[line] != NONE
            && scratch.forward[line] == scratch.backward[line])
            .then(|| scratch.candidates[scratch.forward[line] as usize]);
        let (kind, identity_address, identity_id, secondary) = match paired {
            Some(source) => (
                source.kind,
                source.identity,
                source.identity_id,
                source.secondary,
            ),
            None => (Kind::Unpaired, None, 0, 0),
        };
        let identity = match identity_address {
            Some(address) => arena.copy(memory, address)?,
            None => Span::default(),
        };
        let text = if kind == Kind::Unpaired {
            arena.copy(memory, at(child.pointer, cmodifier::DISPLAY_NAME)?)?
        } else {
            Span::default()
        };
        let fits = arena.fits;
        for (value, key) in child.tracked.iter().zip(context.keys.keys) {
            let (Some(key), true) = (key, *value != 0) else {
                continue;
            };
            if fits {
                lines[line_start + len] = LineRow {
                    country,
                    line: line as u32,
                    key,
                    kind,
                    identity,
                    identity_id,
                    secondary,
                    value: *value,
                    text,
                };
                len += 1;
            }
        }
    }
    Ok(if arena.fits {
        CountryWalk {
            lines: Outcome::Rows { len },
            bytes_len: arena.used - byte_start,
            required_bytes: arena.required - byte_start,
        }
    } else {
        CountryWalk {
            lines: Outcome::Truncated {
                len: 0,
                unread_rows: wanted,
            },
            bytes_len: 0,
            required_bytes: arena.required - byte_start,
        }
    })
}

/// The walk's string arena for one country's lines.
struct Arena<'a> {
    bytes: Buffer<'a, u8>,
    used: usize,
    required: usize,
    fits: bool,
}

impl Arena<'_> {
    /// Copies an exact std::string; once anything does not fit, only the
    /// requirement grows.
    fn copy(&mut self, memory: &impl Memory, address: usize) -> Result<Span, &'static str> {
        let descriptor = Descriptor::read(memory, address)?;
        let start = self.used;
        self.required = self
            .required
            .checked_add(descriptor.len)
            .ok_or("stability line bytes overflow")?;
        if self.fits
            && (matches!(&self.bytes, Buffer::Growing(_)) || self.bytes.ensure(self.required)?)
        {
            descriptor.copy_buffer(memory, &mut self.bytes, start)?;
            self.used += descriptor.len;
        } else {
            self.fits = false;
        }
        Ok(Span {
            start,
            len: descriptor.len,
        })
    }
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("country stability address overflow")
}
fn pointer_from(bytes: &[u8]) -> Result<usize, &'static str> {
    usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
        .ok()
        .filter(|p| *p != 0)
        .ok_or("country stability pointer null or invalid")
}
fn pointer(memory: &impl Memory, address: usize) -> Result<usize, &'static str> {
    pointer_from(&read::<[u8; 8]>(memory, address).ok_or("country stability pointer unreadable")?)
}
fn i64_at(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
fn pointer_pages(
    memory: &impl Memory,
    array: Array,
    mut visit: impl FnMut(usize) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let mut bytes = [0; 64 * 8];
    let mut first = 0;
    while first < array.count {
        let count = (array.count - first).min(64);
        if !read_slice(
            memory,
            array.element(first, 8).ok_or("pointer page overflow")?,
            &mut bytes[..count as usize * 8],
        ) {
            return Err("country stability pointers unreadable");
        }
        for p in bytes[..count as usize * 8].chunks_exact(8) {
            visit(pointer_from(p)?)?;
        }
        first += count;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
