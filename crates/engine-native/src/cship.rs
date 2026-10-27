//! Ship fields relative to the primary pointer. Strength, organisation and
//! experience retain signed i64 values through setters and serialization.

use crate::{
    Memory, cequipment_type, cequipment_variant, cequipment_variant_pool, cname_group_member,
    ctask_force, cunit, msvc_string::Descriptor, read, recent::Recent,
};

mod air_capacity;
mod naming;
pub use naming::{NamingOperands, naming_growing};
mod gathered;
mod hull_variant;
mod naval_power;
pub use air_capacity::{MODULE_PREDICATE, air_capacity_kind};
pub use gathered::{
    GatheredMemory, GatheredObject, HeldShip, HeldVariant, Lookup, Served, ShipFields,
    SwitchedObject, TaskForceFields,
};
pub use hull_variant::{HullVariantInputs, read_hull_variant};
pub use naval_power::{NavalPowerCache, read_naval_power};

/// CShip primary vtable.
pub const VTABLE_RVA: usize = 0x0295_7f28;
/// CShip reference handle {u32 type, u32 id}.
pub const HANDLE: usize = 0x08;
/// CShip reference type 51.
pub const HANDLE_TYPE: u32 = 51;
/// Stored signed acquired crew, independent of equipment demand.
pub const ACQUIRED_MANPOWER: usize = 0x808;
/// Sunk-convoy tally, signed i32, serialized with negative values intact.
pub const SUNK_CONVOYS: usize = 0x3c;
/// Embedded equipment-variant pool used by model selection.
pub const EQUIPMENT_POOL: usize = 0x40;
/// CShip class-database id, i32.
pub const CLASS_DATABASE_ID: usize = 0x88;
/// Embedded subunit definition. Copy construction installs the ordinary
/// definition vtable even when the source is the null singleton. Copy
/// assignment preserves its name and index.
pub const SUBUNIT_DEFINITION: usize = 0x80;
/// Signed definition index used by composition. Refit, deployment and loading
/// copy the definition pair unchanged; serialization writes its token only.
pub const SUBUNIT_INDEX: usize = SUBUNIT_DEFINITION + crate::csub_unit_definition::DATABASE_INDEX;
/// Definition-token string.
pub const DEFINITION: usize = 0x98;
/// Category enum read as u32.
pub const CATEGORY: usize = 0xe0;
/// Fallback hull sprite MSVC string.
pub const FALLBACK_SPRITE: usize = 0xe8;
/// Category flags selecting formation from bits 8, 7 and 39.
pub const FORMATION_FLAGS: usize = 0x628;
/// Definition-valid byte tested before the variant entity ladder.
pub const DEFINITION_VALID: usize = 0x90;
/// Signed fixed5 HP maximum clamped against full strength.
pub const MAXIMUM_STRENGTH: usize = 0x308;
/// Signed fixed5 base organisation combined with effective modifiers.
pub const BASE_ORGANISATION: usize = 0x300;
/// Nullable CAirBase pointer.
pub const CARRIER_AIR_BASE: usize = 0x730;
/// Signed fixed5 anti-air base combined with effective modifiers for fresh
/// naval-window totals.
pub const ANTI_AIR_BASE: usize = 0x218;
/// Entity model number, i32, converted as a signed decimal.
pub const MODEL_NUMBER: usize = 0x810;
/// Name-group member pointer. Naming returns its retained cache without
/// refreshing. Serialization writes naming operands without updating the cache;
/// a saved generated name cannot independently validate live cache freshness.
pub const NAME_MEMBER: usize = 0x818;
/// Inline fixed-reader sprite storage. Worker-owned text grows from the
/// original descriptor when its complete payload needs more room.
pub const MODEL_TEXT_CAPACITY: usize = 256;
/// CShip signed i64 strength.
pub const STRENGTH: usize = 0x6f8;
/// CShip signed i64 organisation.
pub const ORGANISATION: usize = 0x700;
/// CShip signed i64 experience.
pub const EXPERIENCE: usize = 0x708;
/// CShip CTaskForce pointer.
pub const TASK_FORCE: usize = 0x728;
/// CShip allocation size.
pub const SIZE: usize = 0x938;
/// Ship constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00c2_fea0;
/// Shared type-0x33 unit-ID high-water counter, signed i32, serialized as
/// `unit` and restored on load. Divisions share this type with ships.
pub const NEXT_ID_RVA: usize = 0x0308_725c;
/// Scalar fixed5 scale for signed i64 serialization.
pub const FIXED_SCALE: i64 = 100_000;

fn name_member(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    handle: u64,
) -> Result<usize, &'static str> {
    if handle == 0 || handle as u32 != HANDLE_TYPE {
        return Err("ship cached name reference invalid");
    }
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("ship cached name module address overflow")?;
    let identity =
        read::<[u8; 16]>(memory, pointer).ok_or("ship cached name identity unreadable")?;
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64
        || u64::from_le_bytes(identity[8..].try_into().unwrap()) != handle
    {
        return Err("ship cached name identity differs from reference");
    }
    pointer
        .checked_add(NAME_MEMBER)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|address| usize::try_from(address).ok())
        .filter(|address| *address != 0)
        .ok_or("ship cached name member pointer unreadable or null")
}

/// Copies the exact current peaceful or combat ship-name cache. The selected full reference and
/// primary vtable are authenticated before following the name member (`NAME_MEMBER`); the existing
/// NameGroupMember reader owns string guards and arena demand. Its successful span is uncommitted,
/// including a valid empty cache. No generator, combat snapshot, runtime type gate or callback
/// allocation is used.
pub fn cached_name(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    handle: u64,
    names: &mut cname_group_member::Names<'_>,
) -> Result<Option<cname_group_member::Span>, &'static str> {
    let member = name_member(memory, base, pointer, handle)?;
    cname_group_member::cached_name(memory, base, member, names)
}

pub(crate) fn cached_name_buffer(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    handle: u64,
    names: &mut cname_group_member::NameArena<'_>,
) -> Result<Option<cname_group_member::Span>, &'static str> {
    let member = name_member(memory, base, pointer, handle)?;
    cname_group_member::cached_name_buffer(memory, base, member, names)
}

/// Worker-only copy of the retained name-group member cache (`cname_group_member::CACHED_NAME`).
/// Growth continues from the original descriptor without retrying validation.
pub fn cached_name_growing(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    handle: u64,
    bytes: &mut Vec<u8>,
    len: &mut usize,
    required: &mut usize,
) -> Result<Option<cname_group_member::Span>, &'static str> {
    let member = name_member(memory, base, pointer, handle)?;
    cname_group_member::cached_name_growing(memory, base, member, bytes, len, required)
}

/// Same full-reference/member/cache guards as [`cached_name`], with no heap
/// text payload copy. Used only for an off-hour sparse pride-name byte hint.
pub fn cached_name_demand(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    handle: u64,
) -> Result<usize, &'static str> {
    let member = name_member(memory, base, pointer, handle)?;
    cname_group_member::cached_name_demand(memory, base, member)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub name: Option<cname_group_member::Span>,
    pub naming: Option<NamingOperands>,
    /// Full native reference, type in the low u32 and id in the high u32.
    pub id: u64,
    pub task_force: u64,
    /// Native fixed5 current hit points, not a normalized strength percentage.
    pub strength: i64,
    pub organisation: i64,
    pub experience: i64,
    pub category: u32,
    pub sunk_convoys: i32,
    pub class_id: i32,
    pub acquired_manpower: Option<i32>,
    pub required_manpower: Option<i32>,
    /// Position in the task force's ship vector (`ctask_force::SHIPS`) before any replay key sorting.
    pub roster_ordinal: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ModelRow {
    pub ship: u64,
    pub variant: u64,
    pub hull_sprite: ModelText,
    pub fallback_sprite: ModelText,
    pub model_number: i32,
    pub formation_flags: u64,
    pub roster_ordinal: u32,
    pub definition_valid: u8,
    pub carrier_air_base: u64,
    pub maximum_strength: i64,
    pub anti_air_base: i64,
    pub maximum_organisation: i64,
    /// Existing maximum-organisation observation; no independent native read.
    pub has_criticals: Option<u32>,
    /// Independently certified carrier-base subtype; absent for non-carriers.
    pub air_capacity_kind: Option<u32>,
    pub positive_equipment_flags: u64,
    /// Independent guarded suffix; None never invalidates the model prefix.
    pub subunit_index: Option<i32>,
    /// Current selected-variant tuple; acquired independently after max-org.
    pub hull_variant: Option<HullVariantInputs>,
    /// Independently certified current design dominance operands.
    pub naval_power: Option<[i64; 5]>,
}

/// Inline fixed-reader text or a complete worker-owned native string payload.
/// The inline capacity remains a fixture/callback bound, not a worker limit.
#[derive(Debug, PartialEq, Eq)]
pub struct ModelText {
    pub bytes: [u8; MODEL_TEXT_CAPACITY],
    pub len: usize,
    dynamic: Option<Vec<u8>>,
}

impl Default for ModelText {
    fn default() -> Self {
        Self {
            bytes: [0; MODEL_TEXT_CAPACITY],
            len: 0,
            dynamic: None,
        }
    }
}

impl Clone for ModelText {
    fn clone(&self) -> Self {
        self.try_copy().unwrap_or_default()
    }
}

impl ModelText {
    pub fn as_bytes(&self) -> &[u8] {
        match &self.dynamic {
            Some(bytes) => bytes,
            None => &self.bytes[..self.len],
        }
    }
    fn try_copy(&self) -> Result<Self, &'static str> {
        Ok(Self {
            bytes: self.bytes,
            len: self.len,
            dynamic: self
                .dynamic
                .as_ref()
                .map(|text| crate::allocation::copy_vec(text))
                .transpose()
                .map_err(|_| "ship model sprite allocation failed")?,
        })
    }
    pub fn allocated_bytes(&self) -> usize {
        self.dynamic.as_ref().map_or(0, Vec::capacity)
    }
}

/// Completed equipment-definition observations within one stopped capture.
/// A fixed reader keeps the last definition; a worker keeps every completed
/// definition it reached, keyed by its exact (module, definition, token)
/// gather, because ships of one task force alternate hull definitions.
/// Reset before each capture; variant and ship observations remain independent.
#[derive(Default)]
pub struct ModelMemo {
    growing: bool,
    hull: Option<ModelHull>,
    hulls: crate::combat_sources::Map<HullKey, ModelHull>,
    hull_variant: hull_variant::Memo,
}

impl ModelMemo {
    pub fn enable_worker_growth(&mut self) {
        self.growing = true;
    }

    /// Forget every observation (and every native pointer key) for the next
    /// capture, keeping worker storage. Growth must be enabled again.
    pub fn reset(&mut self) {
        self.growing = false;
        self.hull = None;
        self.hulls.clear();
        self.hull_variant.reset();
    }

    /// Retained memo storage, excluding text payloads already counted by rows.
    pub fn allocated_bytes(&self) -> usize {
        self.hulls.capacity() * std::mem::size_of::<(HullKey, ModelHull)>()
            + self
                .hulls
                .values()
                .map(|hull| hull.sprite.allocated_bytes())
                .sum::<usize>()
            + self.hull_variant.allocated_bytes()
    }

    fn hull(&self, key: HullKey) -> Option<&ModelHull> {
        self.hull
            .as_ref()
            .filter(|hull| hull.key == key)
            .or_else(|| self.hulls.get(&key))
    }

    /// Only a complete successful observation is kept. Worker storage grows
    /// per reached definition; an allocation failure (already terminal for
    /// the capture) only forgoes the reuse.
    fn remember_hull(&mut self, hull: ModelHull) {
        if !self.growing {
            self.hull = Some(hull);
        } else if crate::allocation::reserve_map(&mut self.hulls, 1).is_ok() {
            self.hulls.insert(hull.key, hull);
        }
    }
}

/// Module base, equipment definition pointer and its embedded token.
type HullKey = (usize, usize, [u8; 4]);

struct ModelHull {
    key: HullKey,
    sprite: ModelText,
    native_null: bool,
}

/// Read the model inputs from an existing authenticated ship census. Identity
/// and the task-force backlink are checked again for this independent table.
/// The caller owns the current source census and row capacity.
pub fn read_model(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    source: &Row,
    memo: &mut ModelMemo,
) -> Result<ModelRow, &'static str> {
    read_model_with_selection(memory, base, pointer, source, memo).map(|(row, _)| row)
}

/// Callback-local selected address and its completed authenticated observation.
/// Never retain this source past the current stopped census, store it in model
/// rows, or serialize it. Its private fields are minted only on model success.
pub struct SelectedModelVariant {
    pointer: usize,
    handle: u64,
    definition: usize,
    token: [u8; 4],
}

/// The original model reader with a transient selected-variant companion.
/// Ordinary definitions keep their reads, memo admission and first failure.
/// A registered variant's actual null CType singleton is separately admitted;
/// empty selected pools retain variant0 and have no companion.
/// Optional tuple acquisition belongs after the original maximum-org success.
pub fn read_model_with_selection<M: Memory>(
    memory: &M,
    base: usize,
    pointer: usize,
    source: &Row,
    memo: &mut ModelMemo,
) -> Result<(ModelRow, Option<SelectedModelVariant>), &'static str> {
    read_model_observed_inner(memory, base, pointer, source, memo)
}

fn read_model_observed_inner(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    source: &Row,
    memo: &mut ModelMemo,
) -> Result<(ModelRow, Option<SelectedModelVariant>), &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("ship model address overflow")
    };
    let mut identity = [0; 16];
    let mut parent = [0; 8];
    let mut flags = [0; 8];
    let mut model = [0; 4];
    let completed = memory.copy_scattered_prefetched(
        [
            pointer,
            at(TASK_FORCE)?,
            at(FORMATION_FLAGS)?,
            at(MODEL_NUMBER)?,
        ],
        &mut [&mut identity, &mut parent, &mut flags, &mut model],
    );
    if completed != 4 {
        return Err("ship model fields unreadable");
    }
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("ship model module overflow")?;
    let ship = u64::from_le_bytes(identity[8..16].try_into().unwrap());
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64
        || ship != source.id
        || ship as u32 != HANDLE_TYPE
    {
        return Err("ship model identity differs from census");
    }
    let parent = usize::try_from(u64::from_le_bytes(parent))
        .ok()
        .filter(|pointer| *pointer != 0)
        .ok_or("ship model task force pointer invalid")?;
    let mut parent_vtable = [0; 8];
    let mut parent_handle = [0; 8];
    if memory.copy_scattered_prefetched(
        [
            parent,
            parent
                .checked_add(cunit::HANDLE)
                .ok_or("ship model task force address overflow")?,
        ],
        &mut [&mut parent_vtable, &mut parent_handle],
    ) != 2
    {
        return Err("ship model task force identity unreadable");
    }
    if u64::from_le_bytes(parent_vtable)
        != base
            .checked_add(ctask_force::VTABLE_RVA)
            .ok_or("ship model module overflow")? as u64
        || u64::from_le_bytes(parent_handle) != source.task_force
        || source.task_force == 0
    {
        return Err("ship model task force differs from census");
    }
    let (selected, positive_equipment_flags) =
        cequipment_variant_pool::model_selection(memory, base, at(EQUIPMENT_POOL)?)?;
    let fallback_sprite = model_text(memory, at(FALLBACK_SPRITE)?, memo.growing)?;
    let (definition_valid, maximum_strength, air, anti_air_base) =
        read_model_counters(memory, pointer)?;
    let mut row = ModelRow {
        ship,
        positive_equipment_flags,
        fallback_sprite,
        model_number: i32::from_le_bytes(model),
        formation_flags: u64::from_le_bytes(flags),
        roster_ordinal: source.roster_ordinal,
        definition_valid,
        maximum_strength,
        anti_air_base,
        ..ModelRow::default()
    };
    row.carrier_air_base = transport_carrier_base(memory, base, ship, air)?.unwrap_or(0);
    let mut selected_variant = None;
    if selected != 0 {
        let selected_at = |offset| {
            selected
                .checked_add(offset)
                .ok_or("ship model variant address overflow")
        };
        let mut identity = [0; 16];
        let mut token = [0; 4];
        let mut definition = [0; 8];
        if memory.copy_scattered_prefetched(
            [
                selected,
                selected_at(cequipment_variant::EQUIPMENT_TOKEN)?,
                selected_at(cequipment_variant::DEFINITION)?,
            ],
            &mut [&mut identity, &mut token, &mut definition],
        ) != 3
        {
            return Err("ship model variant fields unreadable");
        }
        row.variant = cequipment_variant::identity_from_header(
            base.checked_add(cequipment_variant::VTABLE_RVA)
                .ok_or("ship model module overflow")?,
            identity,
        )?;
        let definition = usize::try_from(u64::from_le_bytes(definition))
            .ok()
            .filter(|pointer| *pointer != 0)
            .ok_or("ship model equipment definition pointer invalid")?;
        // CEquipmentType clones can share tokens. Match the actual definition
        // from this variant's fresh authenticated gather, not its token alone.
        // The stopped capture owns this complete successful observation; no
        // failed identity, descriptor, terminator or payload enters the memo.
        if let Some(hull) = memo.hull((base, definition, token)) {
            if hull.native_null && row.variant >> 32 == 0 {
                return Err("ship model equipment definition identity mismatch");
            }
            row.hull_sprite = hull.sprite.try_copy()?;
        } else {
            let definition_identity = read::<[u8; 12]>(memory, definition)
                .ok_or("ship model equipment definition identity unreadable")?;
            let ordinary = u64::from_le_bytes(definition_identity[..8].try_into().unwrap())
                == base
                    .checked_add(cequipment_type::VTABLE_RVA)
                    .ok_or("ship model module overflow")? as u64;
            if definition_identity[8..12] != token
                || (!ordinary
                    && (row.variant >> 32 == 0
                        || !hull_variant::native_null_definition(
                            memory,
                            base,
                            definition,
                            definition_identity,
                        )))
            {
                return Err("ship model equipment definition identity mismatch");
            }
            row.hull_sprite = model_text(
                memory,
                definition
                    .checked_add(cequipment_type::HULL_SPRITE)
                    .ok_or("ship model equipment sprite address overflow")?,
                memo.growing,
            )?;
            memo.remember_hull(ModelHull {
                key: (base, definition, token),
                sprite: row.hull_sprite.try_copy()?,
                native_null: !ordinary,
            });
        }
        selected_variant = Some(SelectedModelVariant {
            pointer: selected,
            handle: row.variant,
            definition,
            token,
        });
    }
    memo.hull_variant
        .retain_source(base, selected_variant.as_ref());
    Ok((row, selected_variant))
}

/// Read the composition index after `read_model` and the original maximum-org
/// capture have succeeded for this same admitted ship/census observation.
/// The embedded vtable/token authenticate the copied definition independently;
/// null definitions retain their ordinary vtable, token165 and known index0.
/// At most two guarded fixed spans (12+4 bytes) are read. This optional suffix
/// must not consume the original model walk's read budget or retry a failure.
pub fn read_model_subunit_index(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    source: &Row,
) -> Result<i32, &'static str> {
    let definition = pointer
        .checked_add(SUBUNIT_DEFINITION)
        .filter(|_| pointer != 0)
        .ok_or("ship subunit address overflow")?;
    let header = read::<[u8; 12]>(memory, definition).ok_or("ship subunit identity unreadable")?;
    let expected = base
        .checked_add(crate::csub_unit_definition::VTABLE_RVA)
        .ok_or("ship subunit module overflow")? as u64;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected
        || u32::from_le_bytes(header[8..].try_into().unwrap()) != source.class_id as u32
    {
        return Err("ship subunit identity differs from census");
    }
    let index = pointer
        .checked_add(SUBUNIT_INDEX)
        .ok_or("ship subunit index address overflow")?;
    read(memory, index).ok_or("ship subunit index unreadable")
}

/// Four unconditional counter observations. No value predicate lies between
/// them; the completed prefix retains each diagnostic.
fn read_model_counters(
    memory: &impl Memory,
    pointer: usize,
) -> Result<(u8, i64, u64, i64), &'static str> {
    let addresses = [
        DEFINITION_VALID,
        MAXIMUM_STRENGTH,
        CARRIER_AIR_BASE,
        ANTI_AIR_BASE,
    ]
    .map(|offset| pointer.checked_add(offset));
    let [
        Some(valid_at),
        Some(strength_at),
        Some(air_at),
        Some(anti_air_at),
    ] = addresses
    else {
        // Decide before copying: a later address overflow must not suppress a
        // readable earlier field or replace its possible read-failure reason.
        let at = |offset| {
            pointer
                .checked_add(offset)
                .ok_or("ship model address overflow")
        };
        let result = (|| {
            Ok((
                read(memory, at(DEFINITION_VALID)?)
                    .ok_or("ship definition-valid byte unreadable")?,
                read(memory, at(MAXIMUM_STRENGTH)?).ok_or("ship maximum strength unreadable")?,
                read(memory, at(CARRIER_AIR_BASE)?).ok_or("ship carrier air base unreadable")?,
                read(memory, at(ANTI_AIR_BASE)?).ok_or("ship anti-air base unreadable")?,
            ))
        })();
        return result;
    };
    let mut valid = [0; 1];
    let mut strength = [0; 8];
    let mut air = [0; 8];
    let mut anti_air = [0; 8];
    let completed = memory.copy_scattered_prefetched(
        [valid_at, strength_at, air_at, anti_air_at],
        &mut [&mut valid, &mut strength, &mut air, &mut anti_air],
    );
    if completed != 4 {
        return Err([
            "ship definition-valid byte unreadable",
            "ship maximum strength unreadable",
            "ship carrier air base unreadable",
            "ship anti-air base unreadable",
        ][completed]);
    }
    Ok((
        valid[0],
        i64::from_le_bytes(strength),
        u64::from_le_bytes(air),
        i64::from_le_bytes(anti_air),
    ))
}

fn model_text(
    memory: &impl Memory,
    address: usize,
    growing: bool,
) -> Result<ModelText, &'static str> {
    let descriptor = Descriptor::read(memory, address)?;
    if descriptor.len > MODEL_TEXT_CAPACITY {
        if !growing {
            return Err("ship model sprite exceeds owned capacity");
        }
        // The retained sprite descriptor sizes its payload from the original
        // observation rather than a collector text ceiling.
        let mut bytes = Vec::new();
        descriptor.copy_buffer(memory, &mut crate::walk::Buffer::Growing(&mut bytes), 0)?;
        bytes.truncate(descriptor.len);
        return Ok(ModelText {
            len: descriptor.len,
            dynamic: Some(bytes),
            ..ModelText::default()
        });
    }
    let mut text = ModelText {
        len: descriptor.len,
        ..ModelText::default()
    };
    descriptor.copy(memory, &mut text.bytes)?;
    Ok(text)
}

fn transport_carrier_base(
    memory: &impl Memory,
    base: usize,
    ship: u64,
    raw: u64,
) -> Result<Option<u64>, &'static str> {
    if raw == 0 {
        return Ok(None);
    }
    let air = usize::try_from(raw).map_err(|_| "ship carrier air base pointer overflow")?;
    if read::<u64>(memory, air)
        != base
            .checked_add(crate::cair_base::VTABLE_RVA)
            .map(|n| n as u64)
    {
        return Err("ship carrier air base identity invalid");
    }
    let handle = air
        .checked_add(crate::cair_base::HANDLE)
        .and_then(|at| read::<u64>(memory, at))
        .ok_or("ship air base handle unreadable")?;
    if handle >> 32 == 0
        || handle as u32 != crate::cair_base::TYPE
        || air
            .checked_add(crate::cair_base::HOST_REFERENCE)
            .and_then(|at| read::<u64>(memory, at))
            != Some(ship)
    {
        return Err("ship carrier air base has null handle");
    }
    Ok(Some(handle))
}

/// Cache hints for the census fields [`read_row`] copies from the ship at
/// `pointer`, ahead of that read: no read, no fault, no value.
pub(crate) fn prefetch_row(pointer: usize) {
    for (offset, len) in [
        (0, 16),
        (STRENGTH, 24),
        (CATEGORY, 4),
        (SUNK_CONVOYS, 4),
        (CLASS_DATABASE_ID, 4),
        (TASK_FORCE, 8),
    ] {
        crate::prefetch::span(pointer.wrapping_add(offset), len);
    }
}

/// The census fields of [`read_row`] as one record per ship, in its read
/// order: vtable, handle, strength/organisation/experience, category, sunk
/// convoys, class and parent task force. A run of ships is read with one
/// guarded `copy_records` over their pointers ([`crate::cfleet`]).
pub(crate) const ROW_FIELDS: [crate::memory::RecordField; 7] = {
    use crate::memory::RecordField;
    [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        },
        RecordField {
            source_offset: HANDLE,
            destination_offset: 8,
            len: 8,
        },
        RecordField {
            source_offset: STRENGTH,
            destination_offset: 16,
            len: 24,
        },
        RecordField {
            source_offset: CATEGORY,
            destination_offset: 40,
            len: 4,
        },
        RecordField {
            source_offset: SUNK_CONVOYS,
            destination_offset: 44,
            len: 4,
        },
        RecordField {
            source_offset: CLASS_DATABASE_ID,
            destination_offset: 48,
            len: 4,
        },
        RecordField {
            source_offset: TASK_FORCE,
            destination_offset: 56,
            len: 8,
        },
    ]
};
/// Bytes of one [`ROW_FIELDS`] record.
pub(crate) const ROW_STRIDE: usize = 64;
pub(crate) const ROW_LAYOUT: crate::memory::RecordLayout<'static> =
    crate::memory::RecordLayout::new(&ROW_FIELDS, ROW_STRIDE).expect("valid ship row layout");

/// A ship row from a complete [`ROW_FIELDS`] record, with [`read_row`]'s
/// checks after its guarded copy completed every field: the vtable first,
/// then the handle, then the parent task force.
pub(crate) fn decode_row(
    record: &[u8],
    expected_vtable: u64,
    task_force_pointer: usize,
    task_force: u64,
) -> Result<Row, &'static str> {
    let word = |at: usize| u64::from_le_bytes(record[at..at + 8].try_into().expect("fixed span"));
    let half = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().expect("fixed span"));
    if word(0) != expected_vtable {
        return Err("ship vtable mismatch");
    }
    let id = word(8);
    if id == 0 {
        return Err("non-null ship has null handle");
    }
    if word(56) != task_force_pointer as u64 {
        return Err("ship parent differs from task force roster");
    }
    Ok(Row {
        name: None,
        naming: None,
        id,
        task_force,
        strength: word(16) as i64,
        organisation: word(24) as i64,
        experience: word(32) as i64,
        category: half(40),
        sunk_convoys: half(44) as i32,
        class_id: half(48) as i32,
        acquired_manpower: None,
        required_manpower: None,
        roster_ordinal: 0,
    })
}

pub(crate) fn read_row(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    task_force_pointer: usize,
    task_force: u64,
) -> Result<Row, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    let addresses = [
        0,
        HANDLE,
        STRENGTH,
        CATEGORY,
        SUNK_CONVOYS,
        CLASS_DATABASE_ID,
        TASK_FORCE,
    ]
    .map(|offset| pointer.checked_add(offset));
    let [
        Some(vtable_at),
        Some(handle_at),
        Some(strength_at),
        Some(category_at),
        Some(sunk_at),
        Some(class_at),
        Some(parent_at),
    ] = addresses
    else {
        // An overflowing field keeps the vtable check ahead of it.
        if read::<u64>(memory, pointer) != Some(expected as u64) {
            return Err("ship vtable mismatch");
        }
        return Err("ship address overflow");
    };
    // The vtable and the fields in one guarded call; the vtable decides
    // first, as when it was read alone.
    let mut vtable = [0; 8];
    let mut identity = [0; 8];
    let mut scalars = [0; 24];
    let mut category = [0; 4];
    let mut sunk = [0; 4];
    let mut class = [0; 4];
    let mut parent = [0; 8];
    let completed = memory.copy_scattered(
        [
            vtable_at,
            handle_at,
            strength_at,
            category_at,
            sunk_at,
            class_at,
            parent_at,
        ],
        &mut [
            &mut vtable,
            &mut identity,
            &mut scalars,
            &mut category,
            &mut sunk,
            &mut class,
            &mut parent,
        ],
    );
    if completed == 0 || u64::from_le_bytes(vtable) != expected as u64 {
        return Err("ship vtable mismatch");
    }
    if completed != 7 {
        return Err("ship fields unreadable");
    }
    let id = u64::from_le_bytes(identity);
    if id == 0 {
        return Err("non-null ship has null handle");
    }
    if u64::from_le_bytes(parent) != task_force_pointer as u64 {
        return Err("ship parent differs from task force roster");
    }
    Ok(Row {
        name: None,
        naming: None,
        id,
        task_force,
        strength: i64::from_le_bytes(scalars[0..8].try_into().expect("fixed span")),
        organisation: i64::from_le_bytes(scalars[8..16].try_into().expect("fixed span")),
        experience: i64::from_le_bytes(scalars[16..24].try_into().expect("fixed span")),
        category: u32::from_le_bytes(category),
        sunk_convoys: i32::from_le_bytes(sunk),
        class_id: i32::from_le_bytes(class),
        roster_ordinal: 0,
        acquired_manpower: None,
        required_manpower: None,
    })
}

#[cfg(test)]
mod tests;

/// Per-capture memos for complete shared contributions. Ships from a task force
/// are consecutive in the census, so fixed last-source slots need no allocation
/// or hash lookup. Fresh ship/location/fleet selectors choose every slot. Replace
/// this memo at every hourly capture; none of these mutable game values may be
/// reused beyond that callback. Failed or partial contributions are never stored.
#[derive(Default)]
pub struct OrganisationMemo {
    last: Option<(usize, [i64; 4])>,
    /// (game, task roster owner) -> the country Reader::resolve authenticated.
    owner: Option<(usize, u32, usize)>,
    country: Option<(usize, [i64; 4])>,
    /// Country -> its pride-of-the-fleet ship handle (`ccountry::PRIDE_OF_THE_FLEET`).
    pride: Recent<usize, [u8; 8], RECENT>,
    support: Option<(usize, [i64; 4])>,
    /// HQ influence character reference -> its commander's four role terms,
    /// `None` for an influence ship modifier calculation skips: task forces of
    /// a fleet share their headquarters.
    influences: Recent<u64, Option<[i64; 4]>, 16>,
    /// Province definition -> its terrain.
    terrain: Recent<usize, usize, RECENT>,
    /// Terrain lookup's ocean fallback, content resolved once per capture.
    ocean: Option<usize>,
    /// A task force's ships alternate classes and subunits, so these keep
    /// the recent selectors rather than the last one.
    terrain_class: Recent<(usize, u32), [i64; 4], RECENT>,
    fleet: Option<(usize, Option<usize>, [i64; 4])>,
    leader_class: Recent<(usize, u32), [i64; 4], RECENT>,
    doctrine_keys: crate::naval_ship_modifier_inputs::DoctrineKeys,
    weather: Option<OrganisationWeather>,
    definitions: crate::modifier_definition::DefinitionMemo,
    own: Option<(usize, OrganisationOperands)>,
    /// Current metadata for each presence mask of the maximum's four keys.
    modifier_guards: [Option<u16>; 1 << ORGANISATION_KEYS.len()],
}

/// Recent selectors an [`OrganisationMemo`] keeps.
const RECENT: usize = 8;

impl OrganisationMemo {
    /// The ship-owned operands of the latest successful maximum read.
    pub fn organisation_operands(&self, ship: usize) -> Option<OrganisationOperands> {
        self.own
            .filter(|&(pointer, _)| pointer == ship)
            .map(|(_, operands)| operands)
    }

    /// Serve `task`'s doctrine contribution from an earlier hour's complete
    /// observation that a certificate keeps: the next ship of `task` reads no
    /// doctrine tree.
    pub fn hold_doctrine(&mut self, task: usize, contribution: [i64; 4]) {
        self.last = Some((task, contribution));
    }

    /// Serve `country`'s modifier-tree terms (`ccountry::MODIFIER_ACCUMULATOR`, its four ship
    /// modifier keys) from an earlier hour's complete observation that a certificate keeps: the
    /// next ship or task force of that country reads no accumulator for them.
    pub fn hold_country(&mut self, country: usize, terms: [i64; 4]) {
        self.country = Some((country, terms));
    }

    /// The latest country this capture observed and its modifier-tree terms.
    pub fn country_terms(&self) -> Option<(usize, [i64; 4])> {
        self.country
    }

    /// `task`'s doctrine contribution this capture observed, if complete.
    pub fn doctrine(&self, task: usize) -> Option<[i64; 4]> {
        self.last
            .filter(|&(observed, _)| observed == task)
            .map(|(_, contribution)| contribution)
    }

    /// Serve the location terms of `weather`'s task force from an earlier
    /// hour's complete observation that a certificate keeps; the reader
    /// still compares its sources (game, task force, country, location and
    /// province definition) with the ship's.
    pub fn hold_weather(&mut self, weather: OrganisationWeather) {
        self.weather = Some(weather);
    }

    /// The latest location terms this capture observed, if complete.
    pub fn weather(&self) -> Option<OrganisationWeather> {
        self.weather
    }

    /// The task-force terms ship modifier calculation combined into this
    /// capture's latest maximum for `task`, read for one of its ships: its
    /// owner's country terms and pride ship, doctrine, terrain and location
    /// weather, fleet leader and HQ support. `None` unless every one is
    /// `task`'s. A ship whose own inputs and class selectors are unchanged has
    /// an unchanged maximum while these are equal.
    pub fn task_terms(&self, task: &TaskForceFields) -> Option<TaskTerms> {
        let owner = self.owner.filter(|&(_, id, _)| id == task.roster_owner)?;
        let country = self.country.filter(|&(country, _)| country == owner.2)?.1;
        let location = if task.location == 0 {
            None
        } else {
            let weather = self.weather.filter(|weather| {
                weather.sources[1] == task.pointer && weather.sources[3] == task.location
            })?;
            Some((self.terrain.get(&weather.sources[4])?, weather))
        };
        Some(TaskTerms {
            owner,
            country,
            pride: self.pride.get(&owner.2)?,
            doctrine: self.doctrine(task.pointer)?,
            location,
            fleet: self.fleet.filter(|&(fleet, _, _)| fleet == task.fleet)?,
            support: self.support.filter(|&(owner, _)| owner == task.pointer)?.1,
        })
    }

    /// The country the latest ship's task force named, as resolved.
    pub fn country(&self) -> Option<usize> {
        self.owner.map(|(_, _, country)| country)
    }
}

/// Ship-owned operands the organisation maximum reaches, carried only under
/// the ship and medal-store certificate (`ship_model_mutations`).
/// Selectors remain absent when their native branch did not read them.
/// Modifier values stay raw, with current effective metadata applied on
/// recompute. Only the DLC-set command writes the active modifier mask, and
/// it then always reaches the observed world reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrganisationOperands {
    ship: usize,
    handle: [u8; 8],
    class: Option<u32>,
    subunit: Option<u32>,
    weather_factor: Option<i64>,
    base_organisation: i64,
    modifiers: [[crate::modifier_accumulator::Value; 4]; 2],
    criticals: crate::cpdx_array::Array,
    modifier_guard: u16,
}

impl OrganisationOperands {
    /// A moved certificate requires fresh critical-entry observations even
    /// when the gathered header and the ship's other fields compare equal.
    pub fn has_criticals(&self) -> bool {
        self.criticals.count != 0
    }

    /// Two bits per native key: absent, disabled, enabled, or clamped.
    /// Whole-row carry must compare each present key's current metadata.
    pub fn modifier_guard(&self) -> u16 {
        self.modifier_guard
    }
}

/// Compare a carried ship's modifier eligibility and clamp metadata, once
/// per native key-presence mask in a capture. No absent key is read. The
/// effective-value lookup uses these flags, including the active DLC mask.
pub fn organisation_modifier_guard_matches(
    memory: &impl Memory,
    base: usize,
    guard: u16,
    memo: &mut OrganisationMemo,
) -> Result<bool, &'static str> {
    if guard == 0 {
        return Ok(true);
    }
    let present = (0..ORGANISATION_KEYS.len()).fold(0, |mask, index| {
        mask | (usize::from(guard >> (index * 2) & 3 != 0) << index)
    });
    if let Some(current) = memo.modifier_guards[present] {
        return Ok(current == guard);
    }
    let mut current = 0;
    for (index, key) in ORGANISATION_KEYS.into_iter().enumerate() {
        if guard >> (index * 2) & 3 != 0 {
            let (_, flags) = crate::modifier_definition::effective_observed_memo(
                memory,
                base,
                key,
                0,
                &memo.definitions,
            )?;
            current |= modifier_guard(flags) << (index * 2);
        }
    }
    memo.modifier_guards[present] = Some(current);
    Ok(current == guard)
}

/// A task force's ship modifier terms in one capture
/// ([`OrganisationMemo::task_terms`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskTerms {
    owner: (usize, u32, usize),
    country: [i64; 4],
    pride: [u8; 8],
    doctrine: [i64; 4],
    location: Option<(usize, OrganisationWeather)>,
    fleet: (usize, Option<usize>, [i64; 4]),
    support: [i64; 4],
}

impl TaskTerms {
    /// The fleet leader whose class rows select the commander-class term.
    pub fn leader(&self) -> Option<usize> {
        self.fleet.1
    }

    /// Which terms differ from `other`'s, one bit each: owner (1), country
    /// (2), pride ship (4), doctrine (8), location (16), fleet leader (32)
    /// and HQ support (64). Diagnostics.
    pub fn differences(&self, other: &Self) -> u8 {
        [
            self.owner != other.owner,
            self.country != other.country,
            self.pride != other.pride,
            self.doctrine != other.doctrine,
            self.location != other.location,
            self.fleet != other.fleet,
            self.support != other.support,
        ]
        .into_iter()
        .enumerate()
        .fold(0, |bits, (bit, differs)| bits | (u8::from(differs) << bit))
    }
}

/// Ship modifier calculation's region, province and task terms are shared by ships at the same
/// location. The ship's own weather factor (`gathered::WEATHER_FACTOR`) is deliberately outside
/// this packet. Sources are game, task, country, location and province-definition pointers; this
/// complete observation expires with the existing per-capture memo unless a certificate keeps it
/// ([`OrganisationMemo::hold_weather`]). `region_at` and `province_at` are the weather region and
/// province clock records whose accumulators (the region record's modifier and the clock's night
/// factor) it read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrganisationWeather {
    sources: [usize; 5],
    country_adjustment: i64,
    region: [i64; 4],
    location_and_task: [i64; 4],
    region_at: usize,
    province_at: usize,
}

impl OrganisationWeather {
    /// The task force it was read for.
    pub fn task(&self) -> usize {
        self.sources[1]
    }

    /// The weather region record (an element of the weather manager's region array), whose
    /// accumulator weather updating rebuilds.
    pub fn region(&self) -> usize {
        self.region_at
    }

    /// The province clock record's applied night factor, which day/night updating compares before
    /// rewriting the night factor.
    pub fn night_factor(&self) -> usize {
        self.province_at.saturating_add(0x268)
    }
}

fn read_organisation_weather(
    memory: &impl Memory,
    base: usize,
    ship: usize,
    sources: [usize; 5],
    definitions: &crate::modifier_definition::DefinitionMemo,
) -> Result<(OrganisationWeather, i64), &'static str> {
    read_organisation_weather_with(memory, base, sources, definitions, || {
        crate::cnaval_combatant::field(memory, ship, 0x270)
    })
}

/// [`read_organisation_weather`] with the ship's factor read by `factor` at
/// its place in the walk ([`read_task_terms`] has no ship and reads none).
fn read_organisation_weather_with(
    memory: &impl Memory,
    base: usize,
    sources: [usize; 5],
    definitions: &crate::modifier_definition::DefinitionMemo,
    factor: impl FnOnce() -> Result<i64, &'static str>,
) -> Result<(OrganisationWeather, i64), &'static str> {
    use crate::{cnaval_combatant::field, cpdx_array::Array};
    let [game, task, country, location, definition] = sources;
    let add = |p: usize, o: usize| {
        p.checked_add(o)
            .filter(|_| p != 0)
            .ok_or("ship organisation pointer overflow")
    };
    let values = |address| organisation_modifier_values(memory, base, address, definitions);
    let weather = field::<u64>(memory, game, crate::cgame_state::WEATHER_MANAGER)? as usize;
    if field::<u64>(memory, weather, 0)? as usize != add(base, crate::cweather_manager::VTABLE_RVA)?
    {
        return Err("ship organisation source identity mismatch");
    }
    let region = field::<u32>(memory, definition, 0x58)?;
    let regions = Array::read(memory, add(weather, 0x40)?, 0x160)
        .ok_or("ship organisation weather regions invalid")?;
    let region = regions
        .element(region, 0x160)
        .ok_or("ship organisation weather region missing")?;
    let region_at = region;
    // Keep this per-ship observation before the country/region/province terms,
    // exactly where the original walk observes it; a failed packet is not kept.
    let factor = factor()?;
    let country_adjustment = effective_scalar(
        memory,
        base,
        add(country, 0x5b8)?,
        crate::modifier_keys::NAVY_WEATHER_PENALTY,
        definitions,
    )?
    .wrapping_add(100000)
    .clamp(0, 100000);
    let region = values(add(region, 0x58)?)?;
    let provinces = Array::read(memory, add(weather, 0x370)?, 0x330)
        .ok_or("ship organisation weather provinces invalid")?;
    let province = provinces
        .element(field(memory, location, crate::cprovince::ID)?, 0x330)
        .ok_or("ship organisation weather province missing")?;
    let province_at = province;
    let mut location_and_task = values(add(province, 0x280)?)?;
    for (value, task) in location_and_task.iter_mut().zip(values(add(task, 0x668)?)?) {
        *value = value.wrapping_add(task);
    }
    Ok((
        OrganisationWeather {
            sources,
            country_adjustment,
            region,
            location_and_task,
            region_at,
            province_at,
        },
        factor,
    ))
}

/// Two unconditional native fields with the original scalar observation order.
/// Invalid geometry falls back before any copy, retaining an earlier source
/// failure rather than replacing it with a later null/overflow diagnostic.
fn organisation_pair<const A: usize, const B: usize>(
    memory: &impl Memory,
    first: (usize, usize),
    second: (usize, usize),
) -> Result<([u8; A], [u8; B]), &'static str> {
    use crate::cnaval_combatant::field;
    let address = |(pointer, offset): (usize, usize), len| {
        pointer
            .checked_add(offset)
            .filter(|address| pointer != 0 && address.checked_add(len).is_some())
    };
    let (Some(first_at), Some(second_at)) = (address(first, A), address(second, B)) else {
        return Ok((
            field(memory, first.0, first.1)?,
            field(memory, second.0, second.1)?,
        ));
    };
    let mut first = [0; A];
    let mut second = [0; B];
    if memory.copy_scattered([first_at, second_at], &mut [&mut first, &mut second]) != 2 {
        return Err("naval field unreadable");
    }
    Ok((first, second))
}

/// Task-force doctrine-tree contribution. Zero skips the immutable name
/// dictionary; nonzero terms retain both keys.
fn doctrine_contribution(
    memory: &impl Memory,
    base: usize,
    task: usize,
    country: usize,
    definitions: &crate::modifier_definition::DefinitionMemo,
    keys: &mut crate::naval_ship_modifier_inputs::DoctrineKeys,
) -> Result<[i64; 4], &'static str> {
    use crate::{cnaval_combatant::field, naval_ship_modifier_inputs as sources};
    let add = |p: usize, o: usize| {
        p.checked_add(o)
            .filter(|_| p != 0)
            .ok_or("ship organisation pointer overflow")
    };
    let ptr = |p, o| field::<u64>(memory, p, o).map(|n| n as usize);
    let scalar = |address, key| effective_scalar(memory, base, address, key, definitions);
    let values = |address| organisation_modifier_values(memory, base, address, definitions);
    let accumulate = |total: &mut [i64; 4], v: [i64; 4]| {
        for n in 0..4 {
            total[n] = total[n].wrapping_add(v[n]);
        }
    };
    let mut total = [0_i64; 4];
    let head = ptr(task, 0x640)?;
    if field::<u8>(memory, head, 0x19)? != 1 {
        return Err("ship organisation doctrine sentinel invalid");
    }
    let mut node = ptr(head, 0)?;
    let mut previous = None;
    // Task-force construction initializes the map size; traversal follows
    // successors to the original sentinel.
    let count = field::<u64>(memory, task, 0x648)?;
    let mut visited = 0_u64;
    loop {
        if node == head {
            break;
        }
        if visited == count {
            return Err("ship organisation doctrine size mismatch");
        }
        visited += 1;
        if field::<u8>(memory, node, 0x19)? != 0 {
            return Err("ship organisation doctrine node invalid");
        }
        let selector = field::<i32>(memory, node, 0x20)?;
        if previous.is_some_and(|v| v >= selector) {
            return Err("ship organisation doctrine order invalid");
        }
        previous = Some(selector);
        let raw = values(add(ptr(node, 0x30)?, 0x10)?)?;
        if raw.iter().any(|&v| v != 0) {
            let dynamic = sources::doctrine_keys_kept::<true>(memory, base, selector as u32, keys)?;
            let numerator = scalar(add(country, 0x5b8)?, dynamic[0])?
                .wrapping_add(100000)
                .wrapping_mul(i64::from(field::<i32>(memory, node, 0x2c)?))
                .wrapping_mul(100000)
                / 100000;
            let denominator = i64::from(field::<i32>(memory, node, 0x28)?).wrapping_mul(100000);
            let ratio = if denominator == 0
                || (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
            {
                0
            } else {
                numerator
                    .wrapping_mul(100000)
                    .checked_div(denominator)
                    .ok_or("ship doctrine division overflow")?
                    .clamp(0, 100000)
            };
            let modifier = scalar(add(country, 0x5b8)?, dynamic[1])?.wrapping_add(100000);
            accumulate(
                &mut total,
                raw.map(|v| (v.wrapping_mul(modifier) / 100000).wrapping_mul(ratio) / 100000),
            );
        }
        let right = ptr(node, 0x10)?;
        let mut bounded = false;
        if field::<u8>(memory, right, 0x19)? == 0 {
            node = right;
            for _ in 0..count {
                let left = ptr(node, 0)?;
                if field::<u8>(memory, left, 0x19)? != 0 {
                    bounded = true;
                    break;
                }
                node = left;
            }
        } else {
            for _ in 0..count {
                let parent = ptr(node, 8)?;
                if field::<u8>(memory, parent, 0x19)? != 0 || ptr(parent, 0x10)? != node {
                    node = parent;
                    bounded = true;
                    break;
                }
                node = parent;
            }
        }
        if !bounded {
            return Err("ship organisation doctrine traversal bound exceeded");
        }
    }
    if visited != count {
        return Err("ship organisation doctrine size mismatch");
    }
    Ok(total)
}

/// The task-force ship modifier terms ([`OrganisationMemo::task_terms`]) read
/// without a ship: the task force's owner and its country's terms and pride
/// ship, its doctrine contribution, its location's terrain and weather, its
/// fleet leader and its HQ support, each read and kept in `memo` exactly as
/// [`read_maximum_organisation_memo`] reads and keeps it for the task force's
/// first ship (served from `memo` when kept), so a ship of `task` read later
/// finds them there. `task` is the task force a ship named, gathered. None when
/// a term is missing; an error fails nothing a ship's read would not.
pub fn read_task_terms(
    memory: &impl Memory,
    base: usize,
    task: &TaskForceFields,
    memo: &mut OrganisationMemo,
) -> Result<Option<TaskTerms>, &'static str> {
    use crate::{
        cnaval_combatant::field, cpdx_array::Array, naval_ship_modifier_inputs as sources,
    };
    let definitions = &memo.definitions;
    let add = |p: usize, o: usize| {
        p.checked_add(o)
            .filter(|_| p != 0)
            .ok_or("ship organisation pointer overflow")
    };
    let ptr = |p, o| field::<u64>(memory, p, o).map(|n| n as usize);
    let values = |address| organisation_modifier_values(memory, base, address, definitions);
    let auth = |p, rva| -> Result<(), &'static str> {
        if ptr(p, 0)? != add(base, rva)? {
            Err("ship organisation source identity mismatch")
        } else {
            Ok(())
        }
    };
    let task_at = task.pointer;
    auth(task_at, ctask_force::VTABLE_RVA)?;
    let (game, country_id) = organisation_pair::<8, 4>(
        memory,
        (base, crate::cgame_state::SINGLETON_RVA),
        (task_at, cunit::ROSTER_OWNER),
    )?;
    let game = u64::from_le_bytes(game) as usize;
    let country_id = u32::from_le_bytes(country_id);
    let country = match memo.owner {
        Some((source, owner, country)) if source == game && owner == country_id => country,
        _ => {
            let country = crate::ccountry::Reader::new(memory, base, game)?
                .resolve(country_id)?
                .pointer;
            memo.owner = Some((game, country_id, country));
            country
        }
    };
    if !matches!(memo.country, Some((memo_country, _)) if memo_country == country) {
        let value = values(add(country, 0x5b8)?)?;
        memo.country = Some((country, value));
    }
    if memo.pride.get(&country).is_none() {
        memo.pride
            .insert(country, field::<[u8; 8]>(memory, country, 0x250)?);
    }
    if !matches!(memo.last, Some((memo_task, _)) if memo_task == task_at) {
        let value = doctrine_contribution(
            memory,
            base,
            task_at,
            country,
            definitions,
            &mut memo.doctrine_keys,
        )?;
        memo.last = Some((task_at, value));
    }
    let location = ptr(task_at, cunit::LOCATION)?;
    if location != 0 {
        auth(location, crate::cprovince::VTABLE_RVA)?;
        let definition = ptr(location, 0xc8)?;
        if memo.terrain.get(&definition).is_none() {
            let terrain = sources::terrain_kept(memory, base, definition, &mut memo.ocean)?;
            memo.terrain.insert(definition, terrain);
        }
        let sources = [game, task_at, country, location, definition];
        if memo
            .weather
            .as_ref()
            .is_none_or(|weather| weather.sources != sources)
        {
            let (weather, _) =
                read_organisation_weather_with(memory, base, sources, definitions, || Ok(0))?;
            memo.weather = Some(weather);
        }
    }
    let fleet = ptr(task_at, ctask_force::FLEET)?;
    if !matches!(memo.fleet, Some((source, _, _)) if source == fleet) {
        let leader = if fleet == 0 {
            None
        } else {
            auth(fleet, crate::cfleet::VTABLE_RVA)?;
            crate::creference_registry::resolve(memory, base, field(memory, fleet, 0xb0)?)?
        };
        let value = if let Some(leader) = leader {
            auth(leader, crate::cnavy_leader::VTABLE_RVA)?;
            values(add(leader, 0x418)?)?
        } else {
            [0; 4]
        };
        memo.fleet = Some((fleet, leader, value));
    }
    if !matches!(memo.support, Some((memo_task, _)) if memo_task == task_at) {
        let influences = Array::read(memory, add(task_at, 0x728)?, 8)
            .ok_or("ship organisation array invalid or bound exceeded")?;
        let mut support = [0i64; 4];
        for index in 0..influences.count {
            let wrapper = ptr(
                influences
                    .element(index, 8)
                    .ok_or("ship support address overflow")?,
                0,
            )?;
            let influence = ptr(wrapper, 0x40)?;
            if influence == 0 {
                continue;
            }
            let reference = field(memory, influence, 8)?;
            let terms = match memo.influences.get(&reference) {
                Some(terms) => terms,
                None => {
                    let terms = influence_terms(memory, base, reference, &values)?;
                    memo.influences.insert(reference, terms);
                    terms
                }
            };
            let Some(v) = terms else { continue };
            for n in 0..4 {
                support[n] = support[n].max(v[n]);
            }
        }
        memo.support = Some((task_at, support));
    }
    Ok(memo.task_terms(task))
}

/// An HQ influence's four navy-skill terms (ship modifier support), `None` for
/// one it skips: an unresolved character or leader, or a leader that is not a
/// navy commander.
fn influence_terms(
    memory: &impl Memory,
    base: usize,
    reference: u64,
    values: &impl Fn(usize) -> Result<[i64; 4], &'static str>,
) -> Result<Option<[i64; 4]>, &'static str> {
    use crate::cnaval_combatant::field;
    let add = |p: usize, o: usize| {
        p.checked_add(o)
            .filter(|_| p != 0)
            .ok_or("ship organisation pointer overflow")
    };
    let ptr = |p, o| field::<u64>(memory, p, o).map(|n| n as usize);
    let auth = |p, rva| -> Result<(), &'static str> {
        if ptr(p, 0)? != add(base, rva)? {
            Err("ship organisation source identity mismatch")
        } else {
            Ok(())
        }
    };
    let Some(character) = crate::creference_registry::resolve(memory, base, reference)? else {
        return Ok(None);
    };
    auth(character, crate::ccharacter::VTABLE_RVA)?;
    let direct = ptr(character, crate::ccharacter::UNIT_LEADER)?;
    let leader = if direct != 0 {
        Some(direct)
    } else {
        crate::creference_registry::resolve(memory, base, field(memory, character, 0xc8)?)?
    };
    let Some(leader) = leader else {
        return Ok(None);
    };
    if field::<i32>(memory, leader, crate::cunit_leader::CATEGORY)? != 2 {
        return Ok(None);
    }
    auth(leader, crate::cnavy_leader::VTABLE_RVA)?;
    let mut v = [0i64; 4];
    for offset in [0xf70, 0xf80, 0xfa0, 0xf90] {
        let terms = values(add(ptr(leader, offset)?, 0xe0)?)?;
        for n in 0..4 {
            v[n] = v[n].wrapping_add(terms[n]);
        }
    }
    Ok(Some(v))
}

/// Derive the native fixed5 organisation maximum from all ship modifier
/// sources. Four keys share one source traversal; the fourth is critical-effect
/// scaling. No native function is invoked. Arrays/trees and pointer arithmetic
/// are bounded.
pub fn read_maximum_organisation_memo(
    memory: &impl Memory,
    base: usize,
    ship: usize,
    memo: &mut OrganisationMemo,
) -> Result<i64, &'static str> {
    memo.own = None;
    read_organisation_observation(memory, base, ship, memo)
}

/// Observe sources in the original reader's order, retaining ship-owned
/// operands only after the complete maximum succeeds.
fn read_organisation_observation(
    memory: &impl Memory,
    base: usize,
    ship: usize,
    memo: &mut OrganisationMemo,
) -> Result<i64, &'static str> {
    use crate::{
        cnaval_combatant::field, cpdx_array::Array, naval_ship_modifier_inputs as sources,
    };
    let definitions = &memo.definitions;
    let add = |p: usize, o: usize| {
        p.checked_add(o)
            .filter(|_| p != 0)
            .ok_or("ship organisation pointer overflow")
    };
    let ptr = |p, o| field::<u64>(memory, p, o).map(|n| n as usize);
    let values = |address| organisation_modifier_values(memory, base, address, definitions);
    let array = |p, o, stride| -> Result<Array, &'static str> {
        Array::read(memory, add(p, o)?, stride)
            .ok_or("ship organisation array invalid or bound exceeded")
    };
    let auth = |p, rva| -> Result<(), &'static str> {
        if ptr(p, 0)? != add(base, rva)? {
            Err("ship organisation source identity mismatch")
        } else {
            Ok(())
        }
    };
    auth(ship, VTABLE_RVA)?;
    let task = ptr(ship, TASK_FORCE)?;
    auth(task, ctask_force::VTABLE_RVA)?;
    let (game, country_id) = organisation_pair::<8, 4>(
        memory,
        (base, crate::cgame_state::SINGLETON_RVA),
        (task, cunit::ROSTER_OWNER),
    )?;
    let game = u64::from_le_bytes(game) as usize;
    let country_id = u32::from_le_bytes(country_id);
    // Every ship of a task force names the same roster owner; resolving it
    // again within the stopped capture would authenticate the same object.
    let country = match memo.owner {
        Some((source, owner, country)) if source == game && owner == country_id => country,
        _ => {
            let country = crate::ccountry::Reader::new(memory, base, game)?
                .resolve(country_id)?
                .pointer;
            memo.owner = Some((game, country_id, country));
            country
        }
    };
    let country_values = match memo.country {
        Some((memo_country, value)) if memo_country == country => value,
        _ => {
            let value = values(add(country, 0x5b8)?)?;
            memo.country = Some((country, value));
            value
        }
    };
    let (handle, pride) = match memo.pride.get(&country) {
        Some(pride) => (field::<[u8; 8]>(memory, ship, HANDLE)?, pride),
        None => {
            let pair = organisation_pair::<8, 8>(memory, (ship, HANDLE), (country, 0x250))?;
            memo.pride.insert(country, pair.1);
            pair
        }
    };
    let pride_values = if pride == handle {
        let root = ptr(base, 0x0333_0098)?;
        values(add(ptr(root, 0xb0)?, 0x10)?)?
    } else {
        [0; 4]
    };
    let own = organisation_modifier_observation(memory, base, add(ship, 0x750)?, definitions)?;
    let experience = ptr(ship, 0x8f8)?;
    let medals =
        organisation_modifier_observation(memory, base, add(experience, 0x1b0)?, definitions)?;
    // The doctrine tree belongs to the task force; every ship of it adds the
    // same contribution, so it is walked once per task force.
    let doctrine = match memo.last {
        Some((memo_task, value)) if memo_task == task => value,
        _ => {
            let value = doctrine_contribution(
                memory,
                base,
                task,
                country,
                definitions,
                &mut memo.doctrine_keys,
            )?;
            memo.last = Some((task, value));
            value
        }
    };
    let location = ptr(task, cunit::LOCATION)?;
    let mut class = None;
    let mut weather_factor = None;
    let mut terrain_values = [0; 4];
    let location_terms = if location != 0 {
        auth(location, crate::cprovince::VTABLE_RVA)?;
        let definition = ptr(location, 0xc8)?;
        // Ship modifier calculation selects terrain from the province
        // definition; terrain-class adjustment adds only the ship's class. Both
        // sources are shared until the stopped callback returns, including the
        // complete ocean fallback scan.
        let terrain = match memo.terrain.get(&definition) {
            Some(terrain) => terrain,
            None => {
                let terrain = sources::terrain_kept(memory, base, definition, &mut memo.ocean)?;
                memo.terrain.insert(definition, terrain);
                terrain
            }
        };
        let selector = field(memory, ship, CLASS_DATABASE_ID)?;
        class = Some(selector);
        terrain_values = match memo.terrain_class.get(&(terrain, selector)) {
            Some(value) => value,
            None => {
                let (modifier, _) = sources::terrain_class(memory, terrain, selector)?;
                let value = values(add(modifier, 0x10)?)?;
                memo.terrain_class.insert((terrain, selector), value);
                value
            }
        };
        let sources = [game, task, country, location, definition];
        let factor = if memo
            .weather
            .as_ref()
            .is_some_and(|weather| weather.sources == sources)
        {
            field::<i64>(memory, ship, 0x270)?
        } else {
            let (weather, factor) =
                read_organisation_weather(memory, base, ship, sources, definitions)?;
            memo.weather = Some(weather);
            factor
        };
        weather_factor = Some(factor);
        Some((terrain, memo.weather.unwrap()))
    } else {
        None
    };
    let fleet = ptr(task, ctask_force::FLEET)?;
    // Resolve the current task's fleet leader before reuse. The base fleet-
    // leader contribution has no ship operand.
    let (leader, fleet_values) = match memo.fleet {
        Some((source, leader, value)) if source == fleet => (leader, value),
        _ => {
            let leader = if fleet == 0 {
                None
            } else {
                auth(fleet, crate::cfleet::VTABLE_RVA)?;
                crate::creference_registry::resolve(memory, base, field(memory, fleet, 0xb0)?)?
            };
            let value = if let Some(leader) = leader {
                auth(leader, crate::cnavy_leader::VTABLE_RVA)?;
                values(add(leader, 0x418)?)?
            } else {
                [0; 4]
            };
            memo.fleet = Some((fleet, leader, value));
            (leader, value)
        }
    };
    let mut subunit = None;
    let leader_values = if let Some(leader) = leader {
        let selector = field(memory, ship, 0x60c)?;
        subunit = Some(selector);
        match memo.leader_class.get(&(leader, selector)) {
            Some(value) => value,
            None => {
                let class = sources::selected_class(memory, leader, selector, true)?;
                let value = values(add(class, 0x10)?)?;
                memo.leader_class.insert((leader, selector), value);
                value
            }
        }
    } else {
        [0; 4]
    };
    let criticals = array(ship, 0x918, 16)?;
    let critical = read_organisation_criticals(memory, base, criticals, definitions)?;
    // Each recursive key contribution includes its own unscaled criticals
    // and maximum support.
    let support = match memo.support {
        Some((memo_task, value)) if memo_task == task => value,
        _ => {
            let influences = array(task, 0x728, 8)?;
            let mut support = [0i64; 4];
            for index in 0..influences.count {
                let wrapper = ptr(
                    influences
                        .element(index, 8)
                        .ok_or("ship support address overflow")?,
                    0,
                )?;
                let influence = ptr(wrapper, 0x40)?;
                if influence == 0 {
                    continue;
                }
                let reference = field(memory, influence, 8)?;
                let terms = match memo.influences.get(&reference) {
                    Some(terms) => terms,
                    None => {
                        let terms = influence_terms(memory, base, reference, &values)?;
                        memo.influences.insert(reference, terms);
                        terms
                    }
                };
                let Some(v) = terms else { continue };
                for n in 0..4 {
                    support[n] = support[n].max(v[n]);
                }
            }
            memo.support = Some((task, support));
            support
        }
    };
    let mut operands = OrganisationOperands {
        ship,
        handle,
        class,
        subunit,
        weather_factor,
        base_organisation: 0,
        modifiers: [own.raw, medals.raw],
        criticals,
        modifier_guard: own.guard | medals.guard,
    };
    let terms = TaskTerms {
        owner: (game, country_id, country),
        country: country_values,
        pride,
        doctrine,
        location: location_terms,
        fleet: (fleet, leader, fleet_values),
        support,
    };
    let modifiers = combine_organisation(
        memory,
        base,
        &operands,
        &terms,
        [
            own.values,
            medals.values,
            pride_values,
            terrain_values,
            leader_values,
        ],
        critical,
        definitions,
    )?;
    // The base organisation field keeps the reader's original reach, after
    // both critical-entry passes. Only a complete read supplies a packet.
    operands.base_organisation = field(memory, ship, BASE_ORGANISATION)?;
    let maximum = organisation_maximum(operands.base_organisation, modifiers);
    memo.own = Some((ship, operands));
    Ok(maximum)
}

/// Recompute a certified ship's maximum from this hour's task-force terms.
/// The caller proves the packet's ship identity and ship/medal certificate
/// before calling. The terrain-class, leader-subunit and critical
/// pointees remain hourly reads; only the owned selectors and raw arrays
/// are served. A newly reached selector needs the original reader.
pub fn read_maximum_organisation_from_terms(
    memory: &impl Memory,
    base: usize,
    ship: usize,
    operands: &OrganisationOperands,
    terms: &TaskTerms,
    memo: &mut OrganisationMemo,
) -> Result<Option<i64>, &'static str> {
    use crate::{cnaval_combatant::field, naval_ship_modifier_inputs as sources};
    memo.own = None;
    if operands.ship != ship
        || terms.location.is_some()
            && (operands.class.is_none() || operands.weather_factor.is_none())
        || terms.leader().is_some() && operands.subunit.is_none()
    {
        return Ok(None);
    }
    let definitions = &memo.definitions;
    let add = |pointer: usize, offset: usize| {
        pointer
            .checked_add(offset)
            .filter(|_| pointer != 0)
            .ok_or("ship organisation pointer overflow")
    };
    let values = |address| organisation_modifier_values(memory, base, address, definitions);
    let pride = if operands.handle == terms.pride {
        let root = field::<u64>(memory, base, 0x0333_0098)? as usize;
        let source = field::<u64>(memory, root, 0xb0)? as usize;
        values(add(source, 0x10)?)?
    } else {
        [0; 4]
    };
    let mut own = [[0; 4]; 2];
    let mut guard = 0;
    for (values, raw) in own.iter_mut().zip(operands.modifiers) {
        for (index, ((value, raw), key)) in values
            .iter_mut()
            .zip(raw)
            .zip(ORGANISATION_KEYS)
            .enumerate()
        {
            if raw.present {
                let (effective, flags) = crate::modifier_definition::effective_observed_memo(
                    memory,
                    base,
                    key,
                    raw.raw,
                    definitions,
                )?;
                *value = effective;
                guard |= modifier_guard(flags) << (index * 2);
            }
        }
    }
    let terrain = if let Some((terrain, _)) = terms.location {
        let class = operands.class.expect("a reached class selector");
        match memo.terrain_class.get(&(terrain, class)) {
            Some(value) => value,
            None => {
                let (modifier, _) = sources::terrain_class(memory, terrain, class)?;
                let value = values(add(modifier, 0x10)?)?;
                memo.terrain_class.insert((terrain, class), value);
                value
            }
        }
    } else {
        [0; 4]
    };
    let leader = if let Some(leader) = terms.leader() {
        let subunit = operands.subunit.expect("a reached subunit selector");
        match memo.leader_class.get(&(leader, subunit)) {
            Some(value) => value,
            None => {
                let class = sources::selected_class(memory, leader, subunit, true)?;
                let value = values(add(class, 0x10)?)?;
                memo.leader_class.insert((leader, subunit), value);
                value
            }
        }
    } else {
        [0; 4]
    };
    let critical = read_organisation_criticals(memory, base, operands.criticals, definitions)?;
    let modifiers = combine_organisation(
        memory,
        base,
        operands,
        terms,
        [own[0], own[1], pride, terrain, leader],
        critical,
        definitions,
    )?;
    let maximum = organisation_maximum(operands.base_organisation, modifiers);
    memo.own = Some((
        ship,
        OrganisationOperands {
            modifier_guard: guard,
            ..*operands
        },
    ));
    Ok(Some(maximum))
}

fn read_organisation_criticals(
    memory: &impl Memory,
    base: usize,
    criticals: crate::cpdx_array::Array,
    definitions: &crate::modifier_definition::DefinitionMemo,
) -> Result<[i64; 4], &'static str> {
    let mut critical = [0i64; 4];
    for index in 0..criticals.count {
        let entry = criticals
            .element(index, 16)
            .ok_or("ship critical address overflow")?;
        let (count, pointer) = organisation_pair::<4, 8>(memory, (entry, 8), (entry, 0))?;
        let count = i64::from(i32::from_le_bytes(count));
        let pointer = u64::from_le_bytes(pointer) as usize;
        let address = pointer
            .checked_add(0x98)
            .filter(|_| pointer != 0)
            .ok_or("ship organisation pointer overflow")?;
        let values = organisation_modifier_values(memory, base, address, definitions)?;
        for (critical, value) in critical.iter_mut().zip(values) {
            *critical =
                critical.wrapping_add(value.wrapping_mul(count).wrapping_mul(100000) / 100000);
        }
    }
    Ok(critical)
}

/// The ship modifier source additions, weather scaling, recursive critical
/// scaling and support, shared by the full reader and certified recompute.
fn combine_organisation(
    memory: &impl Memory,
    base: usize,
    operands: &OrganisationOperands,
    terms: &TaskTerms,
    contributions: [[i64; 4]; 5],
    critical: [i64; 4],
    definitions: &crate::modifier_definition::DefinitionMemo,
) -> Result<[i64; 3], &'static str> {
    let [own, medals, pride, terrain, leader] = contributions;
    let mut total = terms.country;
    let accumulate = |total: &mut [i64; 4], values: [i64; 4]| {
        for (total, value) in total.iter_mut().zip(values) {
            *total = total.wrapping_add(value);
        }
    };
    for values in [pride, own, medals, terms.doctrine] {
        accumulate(&mut total, values);
    }
    if let Some((_, weather)) = terms.location {
        accumulate(&mut total, terrain);
        let scale = operands
            .weather_factor
            .expect("a reached weather factor")
            .wrapping_sub(100000)
            .wrapping_add(weather.country_adjustment);
        accumulate(
            &mut total,
            weather
                .region
                .map(|value| value.wrapping_mul(scale) / 100000),
        );
        accumulate(&mut total, weather.location_and_task);
    }
    if terms.leader().is_some() {
        accumulate(&mut total, terms.fleet.2);
        accumulate(&mut total, leader);
    }
    let scale = total[3]
        .wrapping_add(critical[3])
        .wrapping_add(terms.support[3])
        .wrapping_add(100000);
    // Each entry truncates before summing, as the game does; its counts
    // and pointed modifier values are observed again on this second pass.
    for index in 0..operands.criticals.count {
        let entry = operands
            .criticals
            .element(index, 16)
            .ok_or("ship critical address overflow")?;
        let (count, pointer) = organisation_pair::<4, 8>(memory, (entry, 8), (entry, 0))?;
        let count = i64::from(i32::from_le_bytes(count));
        let pointer = u64::from_le_bytes(pointer) as usize;
        let address = pointer
            .checked_add(0x98)
            .filter(|_| pointer != 0)
            .ok_or("ship organisation pointer overflow")?;
        let values = organisation_modifier_values(memory, base, address, definitions)?;
        for n in 0..3 {
            total[n] = total[n].wrapping_add(
                (values[n].wrapping_mul(count).wrapping_mul(100000) / 100000).wrapping_mul(scale)
                    / 100000,
            );
        }
    }
    Ok(std::array::from_fn(|n| {
        total[n].wrapping_add(terms.support[n])
    }))
}

/// Capture the four organisation-modifier operands with one full direct
/// census when needed. Hooks and present-key metadata retain the original
/// navy-organisation, navy-organisation-factor, maximum-organisation and
/// critical-effect-factor order. Direct raw values are reused only within
/// this stopped-callback invocation. Compute one effective accumulator
/// scalar with the metadata memo.
fn effective_scalar(
    memory: &impl Memory,
    base: usize,
    address: usize,
    key: u32,
    definitions: &crate::modifier_definition::DefinitionMemo,
) -> Result<i64, &'static str> {
    let value = crate::modifier_accumulator::lookup_at(memory, address, key)?;
    if value.present {
        crate::modifier_definition::effective_memo(memory, base, key, value.raw, definitions)
    } else {
        Ok(0)
    }
}

fn organisation_modifier_values(
    memory: &impl Memory,
    base: usize,
    address: usize,
    definitions: &crate::modifier_definition::DefinitionMemo,
) -> Result<[i64; 4], &'static str> {
    organisation_modifier_observation(memory, base, address, definitions)
        .map(|observation| observation.values)
}

const ORGANISATION_KEYS: [u32; 4] = [
    crate::modifier_keys::NAVY_ORG,
    crate::modifier_keys::NAVY_ORG_FACTOR,
    crate::modifier_keys::MAX_ORGANISATION,
    crate::modifier_keys::NAVAL_CRITICAL_EFFECT_FACTOR,
];

fn modifier_guard(flags: Option<u32>) -> u16 {
    match flags {
        None => 1,
        Some(flags) if flags & crate::modifier_definition::CLAMP_MAXIMUM != 0 => 3,
        Some(_) => 2,
    }
}

struct OrganisationModifiers {
    values: [i64; 4],
    raw: [crate::modifier_accumulator::Value; 4],
    guard: u16,
}

#[inline]
fn organisation_modifier_observation(
    memory: &impl Memory,
    base: usize,
    address: usize,
    definitions: &crate::modifier_definition::DefinitionMemo,
) -> Result<OrganisationModifiers, &'static str> {
    use crate::modifier_accumulator::{Value, lookup_many_at};

    const SORTED_KEYS: [u32; 4] = [
        crate::modifier_keys::NAVY_ORG,
        crate::modifier_keys::NAVY_ORG_FACTOR,
        crate::modifier_keys::NAVAL_CRITICAL_EFFECT_FACTOR,
        crate::modifier_keys::MAX_ORGANISATION,
    ];
    let mut direct = [Value::default(); 4];
    let mut captured = false;
    let mut values = [0; 4];
    let mut raw_values = [Value::default(); 4];
    let mut guard = 0;
    for (native, (value, index)) in values.iter_mut().zip([0, 1, 3, 2]).enumerate() {
        let key = SORTED_KEYS[index];
        let raw = match memory.modifier_accumulator_value(address, key) {
            Some(result) => result?,
            None => {
                if !captured {
                    // The current census validates every entry, including its
                    // unused suffix, before reusing these raw values.
                    lookup_many_at(memory, address, &SORTED_KEYS, &mut direct)?;
                    captured = true;
                }
                direct[index]
            }
        };
        raw_values[native] = raw;
        if raw.present {
            let (effective, flags) = crate::modifier_definition::effective_observed_memo(
                memory,
                base,
                key,
                raw.raw,
                definitions,
            )?;
            *value = effective;
            guard |= modifier_guard(flags) << (native * 2);
        }
    }
    Ok(OrganisationModifiers {
        values,
        raw: raw_values,
        guard,
    })
}

/// Signed wrapping maximum-organisation arithmetic. The minimum 100 is
/// fixed5, representing 0.001 organisation.
pub fn organisation_maximum(base: i64, modifiers: [i64; 3]) -> i64 {
    (modifiers[1]
        .wrapping_add(modifiers[2])
        .wrapping_add(100000)
        .wrapping_mul(base.wrapping_add(modifiers[0]))
        / 100000)
        .max(100)
}

#[cfg(test)]
mod organisation_tests;

#[cfg(test)]
mod cached_name_tests {
    use super::*;
    use crate::memory::SliceMemory;
    use std::cell::RefCell;

    const SHIP: usize = 0x1000;
    const MEMBER: usize = 0x2000;
    const HEAP: usize = 0x3000;
    const FULL_HANDLE: u64 = (0x8765_4321_u64 << 32) | HANDLE_TYPE as u64;

    fn word(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn fixture(text: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; HEAP + text.len() + 1];
        word(&mut bytes, SHIP, VTABLE_RVA as u64);
        word(&mut bytes, SHIP + HANDLE, FULL_HANDLE);
        word(&mut bytes, SHIP + NAME_MEMBER, MEMBER as u64);
        word(&mut bytes, MEMBER, cname_group_member::VTABLE_RVA as u64);
        // Generator/runtime naming types are outside the retained getter.
        bytes[MEMBER + cname_group_member::TYPE..MEMBER + cname_group_member::TYPE + 4]
            .copy_from_slice(&i32::MIN.to_le_bytes());
        let string = MEMBER + cname_group_member::CACHED_NAME;
        word(&mut bytes, string + 16, text.len() as u64);
        if text.len() <= 15 {
            bytes[string..string + text.len()].copy_from_slice(text);
            word(&mut bytes, string + 24, 15);
        } else {
            word(&mut bytes, string, HEAP as u64);
            word(&mut bytes, string + 24, text.len() as u64);
            bytes[HEAP..HEAP + text.len()].copy_from_slice(text);
        }
        bytes
    }

    struct Tracked<'a> {
        bytes: &'a [u8],
        denied: Option<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Tracked<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            if self
                .denied
                .is_some_and(|at| address <= at && at < address.saturating_add(destination.len()))
            {
                return false;
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }
    fn tracked(bytes: &[u8], denied: Option<usize>) -> Tracked<'_> {
        Tracked {
            bytes,
            denied,
            reads: RefCell::default(),
        }
    }

    #[test]
    fn current_ship_cache_preserves_empty_heap_and_raw_bytes_without_committing() {
        for text in [
            &b""[..],
            &b"Ship\0suffix"[..],
            &b"A peaceful retained ship name"[..],
            &[0xff][..],
        ] {
            let bytes = fixture(text);
            let memory = tracked(&bytes, None);
            let mut arena = vec![0x7f; 3 + text.len()];
            let mut names = cname_group_member::Names {
                bytes: &mut arena,
                len: 3,
                required: 0,
            };
            assert_eq!(
                cached_name(&memory, 0, SHIP, FULL_HANDLE, &mut names),
                Ok(Some((3, text.len())))
            );
            assert_eq!((names.len, names.required), (3, text.len()));
            assert_eq!(&names.bytes[..3], &[0x7f; 3]);
            assert_eq!(&names.bytes[3..], text);
        }
    }

    #[test]
    fn ship_name_rejects_full_identity_before_following_member() {
        let mut bytes = fixture(b"Retained");
        bytes[SHIP] ^= 1;
        let valid = fixture(b"Retained");
        for (bytes, handle, expected_reads) in [
            (&bytes, FULL_HANDLE, vec![(SHIP, 16)]),
            (&valid, FULL_HANDLE ^ (1_u64 << 32), vec![(SHIP, 16)]),
            (&bytes, FULL_HANDLE ^ 1, vec![]),
            (&bytes, 0, vec![]),
        ] {
            let memory = tracked(bytes, None);
            assert!(cached_name_demand(&memory, 0, SHIP, handle).is_err());
            assert_eq!(*memory.reads.borrow(), expected_reads);
        }
        let mut bytes = fixture(b"Retained");
        word(&mut bytes, SHIP + NAME_MEMBER, 0);
        assert_eq!(
            cached_name_demand(&tracked(&bytes, None), 0, SHIP, FULL_HANDLE),
            Err("ship cached name member pointer unreadable or null")
        );
        word(&mut bytes, SHIP + NAME_MEMBER, MEMBER as u64);
        bytes[MEMBER] ^= 1;
        assert_eq!(
            cached_name_demand(&tracked(&bytes, None), 0, SHIP, FULL_HANDLE),
            Err("division cached name member identity unreadable or invalid")
        );
    }

    #[test]
    fn ship_name_hint_and_short_arena_count_only_valid_header_boundary_demand() {
        let text = b"A retained ship name longer than the admitted byte arena";
        let mut bytes = fixture(text);
        let memory = tracked(&bytes, None);
        assert_eq!(
            cached_name_demand(&memory, 0, SHIP, FULL_HANDLE),
            Ok(text.len())
        );
        let heap_reads: Vec<_> = memory
            .reads
            .borrow()
            .iter()
            .copied()
            .filter(|&(address, _)| (HEAP..=HEAP + text.len()).contains(&address))
            .collect();
        assert_eq!(heap_reads, [(HEAP, 1), (HEAP + text.len() - 1, 2)]);
        let mut names = cname_group_member::Names {
            bytes: &mut [],
            len: 0,
            required: 0,
        };
        assert_eq!(
            cached_name(&memory, 0, SHIP, FULL_HANDLE, &mut names),
            Ok(None)
        );
        assert_eq!(names.required, text.len());
        bytes[HEAP + text.len()] = 1;
        let mut names = cname_group_member::Names {
            bytes: &mut [],
            len: 0,
            required: 0,
        };
        assert!(cached_name(&tracked(&bytes, None), 0, SHIP, FULL_HANDLE, &mut names).is_err());
        assert_eq!(names.required, 0);
        bytes[HEAP + text.len()] = 0;
        let mut arena = vec![0; text.len()];
        let mut names = cname_group_member::Names {
            bytes: &mut arena,
            len: 0,
            required: 0,
        };
        assert!(
            cached_name(
                &tracked(&bytes, Some(HEAP + 2)),
                0,
                SHIP,
                FULL_HANDLE,
                &mut names
            )
            .is_err()
        );
        assert_eq!((names.len, names.required), (0, text.len()));
    }
}
