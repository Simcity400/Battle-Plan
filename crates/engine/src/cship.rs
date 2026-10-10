//! Ship fields relative to the primary pointer. Strength, organisation and
//! experience retain signed i64 values through setters and serialization.

use crate::{
    Memory, cequipment_type, cequipment_variant, cequipment_variant_pool, cname_group_member,
    ctask_force, cunit, msvc_string::Descriptor, read,
};

mod strike;
pub use strike::{
    METADATA_CAPACITY as STRIKE_METADATA_CAPACITY, OrganisationAndCooldown, ShipStrikeInputs,
    TorpedoCooldownMemo, read_maximum_organisation_and_torpedo_cooldown, read_strike_bases,
};

mod hull_variant;
pub use hull_variant::{HullVariantInputs, read_hull_variant};

/// CShip primary vtable.
pub const VTABLE_RVA: usize = 0x0294_1748;
/// CShip reference handle {u32 type, u32 id}.
pub const HANDLE: usize = 0x08;
/// CShip reference type 51.
pub const HANDLE_TYPE: u32 = 51;
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
/// CShip signed i64 maximum experience.
pub const MAXIMUM_EXPERIENCE: usize = 0x710;
/// CShip CTaskForce pointer.
pub const TASK_FORCE: usize = 0x728;
/// Scalar slot width.
pub const SCALAR_STORAGE_BYTES: usize = 8;
/// CShip allocation size.
pub const SIZE: usize = 0x938;
/// Ship constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00c2_0760;
/// Scalar-deleting destructor.
pub const DELETING_DESTRUCTOR_RVA: usize = 0x00c2_1580;
/// Reference-ID mint/restore routine raising the shared high-water counter.
pub const REGISTER_ID_RVA: usize = 0x00c2_4b40;
/// Alternate ID mint/restore routine with an additional game-state gate.
pub const REGISTER_ID_ALTERNATE_RVA: usize = 0x00bf_0ae0;
/// Shared type-0x33 unit-ID high-water counter, signed i32, serialized as
/// `unit` and restored on load. Divisions share this type with ships.
pub const NEXT_ID_RVA: usize = 0x0306_f268;
/// Named ship-category values; other category values remain unnamed.
pub const KNOWN_CATEGORIES: [(u32, &str); 7] = [
    (2, "submarine"),
    (3, "destroyer"),
    (5, "light_cruiser"),
    (6, "heavy_cruiser"),
    (8, "battle_cruiser"),
    (9, "battleship"),
    (10, "carrier"),
];
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

/// Copies the exact current peaceful or combat ship-name cache. The selected
/// full reference and primary vtable are authenticated before following+818;
/// the existing NameGroupMember reader owns string guards and arena demand.
/// Its successful span is uncommitted, including a valid empty cache. No
/// generator, combat snapshot, runtime type gate or callback allocation is used.
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

/// Worker-only copy of the retained name-group member cache at +0x60.
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
    /// Position in CTaskForce+348 before any replay key sorting.
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
    pub positive_equipment_flags: u64,
    /// Independent guarded suffix; None never invalidates the model prefix.
    pub subunit_index: Option<i32>,
    /// Current readiness packet; each guarded scalar remains independent.
    pub strike: Option<ShipStrikeInputs>,
    /// Current selected-variant tuple; acquired independently after max-org.
    pub hull_variant: Option<HullVariantInputs>,
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

/// Numerical carrier leaves lent by the original model attempt. `None` is
/// unattempted and `Some(Err)` is an immutable original failure. Flags select
/// the carrier path and its optional base is independently authenticated.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportModelObservation {
    pub identity: Option<Result<(), &'static str>>,
    pub formation_flags: Option<Result<u64, &'static str>>,
    pub positive_equipment_flags: Option<Result<u64, &'static str>>,
    pub carrier_air_base: Option<Result<Option<u64>, &'static str>>,
}

/// One completed equipment-definition observation within a stopped capture.
/// Reset before each capture; variant and ship observations remain independent.
#[derive(Default)]
pub struct ModelMemo {
    growing: bool,
    hull: Option<ModelHull>,
    hull_variant: hull_variant::Memo,
}

impl ModelMemo {
    pub fn enable_worker_growth(&mut self) {
        self.growing = true;
    }
}

struct ModelHull {
    base: usize,
    definition: usize,
    token: [u8; 4],
    sprite: ModelText,
    native_null: bool,
}

struct ModelObservation<'a> {
    transport: &'a mut TransportModelObservation,
    air: &'a mut Option<Result<u64, &'static str>>,
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
    read_model_observed_inner(
        memory,
        None::<&M>,
        base,
        pointer,
        source,
        memo,
        ModelObservation {
            transport: &mut TransportModelObservation::default(),
            air: &mut None,
        },
    )
}

/// The ordinary model pass lends original successful and failed carrier leaves.
/// Unrelated sprite, model-number and maximum-organisation failures cannot
/// erase an already observed flag or carrier identity.
pub fn read_model_with_selection_observed(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    source: &Row,
    memo: &mut ModelMemo,
    observation: &mut TransportModelObservation,
) -> Result<(ModelRow, Option<SelectedModelVariant>), &'static str> {
    read_model_with_selection_transport_observed(
        memory,
        memory,
        base,
        pointer,
        source,
        memo,
        observation,
    )
}

pub fn read_model_with_selection_transport_observed(
    memory: &impl Memory,
    transport_memory: &impl Memory,
    base: usize,
    pointer: usize,
    source: &Row,
    memo: &mut ModelMemo,
    observation: &mut TransportModelObservation,
) -> Result<(ModelRow, Option<SelectedModelVariant>), &'static str> {
    let mut air = None;
    let result = read_model_observed_inner(
        memory,
        Some(transport_memory),
        base,
        pointer,
        source,
        memo,
        ModelObservation {
            transport: observation,
            air: &mut air,
        },
    );
    if observation.identity.is_none() {
        observation.identity = Some(result.as_ref().map(|_| ()).map_err(|reason| *reason));
    }
    if observation.carrier_air_base.is_none()
        && observation.identity == Some(Ok(()))
        && let Some(air) = air
    {
        // The old counter prefix can fail after its carrier pointer was
        // copied. Authenticate that loan once, after the old read finishes.
        observation.carrier_air_base = Some(
            air.and_then(|air| transport_carrier_base(transport_memory, base, source.id, air)),
        );
    }
    result
}

fn read_model_observed_inner(
    memory: &impl Memory,
    transport_memory: Option<&impl Memory>,
    base: usize,
    pointer: usize,
    source: &Row,
    memo: &mut ModelMemo,
    observation: ModelObservation<'_>,
) -> Result<(ModelRow, Option<SelectedModelVariant>), &'static str> {
    let ModelObservation {
        transport: observation,
        air: air_observation,
    } = observation;
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
        // The first two successful spans are independent of model number.
        // The source's original SHIPS authentication owns the parent loan;
        // retain copied flags, and retain a failed flags read as a failure.
        if completed >= 2 {
            let header_valid = base.checked_add(VTABLE_RVA).map(|vtable| vtable as u64)
                == Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
                && u64::from_le_bytes(identity[8..].try_into().unwrap()) == source.id
                && u64::from_le_bytes(parent) != 0;
            observation.identity = Some(if header_valid {
                Ok(())
            } else {
                Err("ship model identity differs from census")
            });
            observation.formation_flags = Some(if completed >= 3 {
                Ok(u64::from_le_bytes(flags))
            } else {
                Err("ship model formation flags unreadable")
            });
        }
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
    observation.identity = Some(Ok(()));
    observation.formation_flags = Some(Ok(u64::from_le_bytes(flags)));
    let selection = if let Some(transport_memory) =
        transport_memory.filter(|_| u64::from_le_bytes(flags) & (1 << 22) == 0)
    {
        let mut flags = Ok(0);
        let result = cequipment_variant_pool::model_selection_transport_observed(
            memory,
            transport_memory,
            base,
            at(EQUIPMENT_POOL)?,
            &mut flags,
        );
        observation.positive_equipment_flags = Some(flags);
        result
    } else {
        let result = cequipment_variant_pool::model_selection(memory, base, at(EQUIPMENT_POOL)?);
        observation.positive_equipment_flags = Some(result.map(|(_, flags)| flags));
        result
    };
    let (selected, positive_equipment_flags) = selection?;
    let fallback_sprite = model_text(memory, at(FALLBACK_SPRITE)?, memo.growing)?;
    let (definition_valid, maximum_strength, air, anti_air_base) =
        read_model_counters_observed(memory, pointer, air_observation)?;
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
    let base_observation = transport_carrier_base(memory, base, ship, air);
    observation.carrier_air_base = Some(base_observation);
    row.carrier_air_base = base_observation?.unwrap_or(0);
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
        // failed identity, descriptor, terminator or payload enters the slot.
        if let Some(hull) = memo.hull.as_ref().filter(|hull| {
            hull.base == base && hull.definition == definition && hull.token == token
        }) {
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
            memo.hull = Some(ModelHull {
                base,
                definition,
                token,
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
#[cfg(test)]
fn read_model_counters(
    memory: &impl Memory,
    pointer: usize,
) -> Result<(u8, i64, u64, i64), &'static str> {
    read_model_counters_observed(memory, pointer, &mut None)
}

fn read_model_counters_observed(
    memory: &impl Memory,
    pointer: usize,
    observation: &mut Option<Result<u64, &'static str>>,
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
                {
                    let raw = read(memory, at(CARRIER_AIR_BASE)?)
                        .ok_or("ship carrier air base unreadable");
                    *observation = Some(raw);
                    raw?
                },
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
    if completed >= 3 {
        *observation = Some(Ok(u64::from_le_bytes(air)));
    } else if completed == 2 {
        *observation = Some(Err("ship carrier air base unreadable"));
    }
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

/// Complete only genuinely unattempted carrier leaves inside the original
/// authenticated SHIPS loan. Original errors, including pool failures, remain
/// immutable. The carrier-flag early-out skips the pool.
pub fn read_transport_model(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    ship: u64,
    mut original: TransportModelObservation,
) -> TransportModelObservation {
    if original.identity.is_some_and(|identity| identity.is_err()) {
        return original;
    }
    original.identity = Some(Ok(()));
    let flags = *original.formation_flags.get_or_insert_with(|| {
        pointer
            .checked_add(FORMATION_FLAGS)
            .and_then(|at| read(memory, at))
            .ok_or("ship transport formation flags unreadable")
    });
    let Ok(flags) = flags else {
        return original;
    };
    let carrier = if flags & (1 << 22) != 0 {
        true
    } else {
        let positive = *original.positive_equipment_flags.get_or_insert_with(|| {
            pointer
                .checked_add(EQUIPMENT_POOL)
                .ok_or("ship transport equipment pool overflow")
                .and_then(|pool| {
                    cequipment_variant_pool::transport_carrier_flags(memory, base, pool)
                })
        });
        let Ok(positive) = positive else {
            return original;
        };
        positive & (1 << 22) != 0 || (flags | positive) & (1 << 37) != 0
    };
    if carrier && original.carrier_air_base.is_none() {
        original.carrier_air_base = Some(
            pointer
                .checked_add(CARRIER_AIR_BASE)
                .and_then(|at| read::<u64>(memory, at))
                .ok_or("ship carrier air base unreadable")
                .and_then(|raw| transport_carrier_base(memory, base, ship, raw)),
        );
    }
    original
}

pub(crate) fn read_row(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    task_force_pointer: usize,
    task_force: u64,
) -> Result<Row, &'static str> {
    let at = |offset| pointer.checked_add(offset).ok_or("ship address overflow");
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("ship vtable mismatch");
    }
    let mut identity = [0; 8];
    let mut scalars = [0; 24];
    let mut category = [0; 4];
    let mut sunk = [0; 4];
    let mut class = [0; 4];
    let mut parent = [0; 8];
    if memory.copy_scattered(
        [
            at(HANDLE)?,
            at(STRENGTH)?,
            at(CATEGORY)?,
            at(SUNK_CONVOYS)?,
            at(CLASS_DATABASE_ID)?,
            at(TASK_FORCE)?,
        ],
        &mut [
            &mut identity,
            &mut scalars,
            &mut category,
            &mut sunk,
            &mut class,
            &mut parent,
        ],
    ) != 6
    {
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
        id,
        task_force,
        strength: i64::from_le_bytes(scalars[0..8].try_into().expect("fixed span")),
        organisation: i64::from_le_bytes(scalars[8..16].try_into().expect("fixed span")),
        experience: i64::from_le_bytes(scalars[16..24].try_into().expect("fixed span")),
        category: u32::from_le_bytes(category),
        sunk_convoys: i32::from_le_bytes(sunk),
        class_id: i32::from_le_bytes(class),
        roster_ordinal: 0,
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
    country: Option<(usize, [i64; 4])>,
    support: Option<(usize, [i64; 4])>,
    terrain: Option<(usize, usize)>,
    terrain_class: Option<(usize, u32, [i64; 4])>,
    fleet: Option<(usize, Option<usize>, [i64; 4])>,
    leader_class: Option<(usize, u32, [i64; 4])>,
    weather: Option<OrganisationWeather>,
    definitions: crate::modifier_definition::DefinitionMemo,
}

/// Region, province and task modifiers shared by ships at the same location.
/// The ship-specific factor stays outside this packet. Game, task, country,
/// location and province-definition observations expire with the capture memo.
struct OrganisationWeather {
    sources: [usize; 5],
    country_adjustment: i64,
    region: [i64; 4],
    location_and_task: [i64; 4],
}

fn read_organisation_weather(
    memory: &impl Memory,
    base: usize,
    ship: usize,
    sources: [usize; 5],
    definitions: &crate::modifier_definition::DefinitionMemo,
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
    // Keep this per-ship observation before the country/region/province terms,
    // exactly where the original walk observes it; a failed packet is not kept.
    let factor = field(memory, ship, 0x270)?;
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
            let dynamic = sources::doctrine_keys::<true>(memory, base, selector as u32)?;
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

/// Derive fixed5 organisation maximum from all effective sources. Four keys
/// share one traversal; the fourth scales critical effects. No game function
/// is invoked, and arrays, trees and address arithmetic remain bounded.
pub fn read_maximum_organisation_memo(
    memory: &impl Memory,
    base: usize,
    ship: usize,
    memo: &mut OrganisationMemo,
) -> Result<i64, &'static str> {
    read_organisation_observation(memory, base, ship, memo).map(|observed| observed.maximum)
}

/// Internal completed observation. The optional strike wrapper consumes it in
/// this same invocation; no source pointer, leader or critical scale is kept
/// in the hourly memo or projected into replay.
struct OrganisationObservation {
    maximum: i64,
    handle: u64,
    task: usize,
    fleet: usize,
    leader: Option<usize>,
    criticals: crate::cpdx_array::Array,
    critical_scale: i64,
}

fn read_organisation_observation(
    memory: &impl Memory,
    base: usize,
    ship: usize,
    memo: &mut OrganisationMemo,
) -> Result<OrganisationObservation, &'static str> {
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
    let accumulate = |total: &mut [i64; 4], v: [i64; 4]| {
        for n in 0..4 {
            total[n] = total[n].wrapping_add(v[n]);
        }
    };
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
    let country = crate::ccountry::Reader::new(memory, base, game)?
        .resolve(country_id)?
        .pointer;
    let mut total = match memo.country {
        Some((memo_country, value)) if memo_country == country => value,
        _ => {
            let value = values(add(country, 0x5b8)?)?;
            memo.country = Some((country, value));
            value
        }
    };
    let (handle, pride) = organisation_pair::<8, 8>(memory, (ship, HANDLE), (country, 0x250))?;
    if pride == handle {
        let root = ptr(base, 0x0331_7cc8)?;
        accumulate(&mut total, values(add(ptr(root, 0xb0)?, 0x10)?)?);
    }
    accumulate(&mut total, values(add(ship, 0x750)?)?);
    let experience = ptr(ship, 0x8f8)?;
    accumulate(&mut total, values(add(experience, 0x1b0)?)?);
    // The doctrine tree belongs to the task force; every ship of it adds the
    // same contribution, so it is walked once per task force.
    let contribution = match memo.last {
        Some((memo_task, value)) if memo_task == task => value,
        _ => {
            let value = doctrine_contribution(memory, base, task, country, definitions)?;
            memo.last = Some((task, value));
            value
        }
    };
    accumulate(&mut total, contribution);
    let location = ptr(task, cunit::LOCATION)?;
    if location != 0 {
        auth(location, crate::cprovince::VTABLE_RVA)?;
        let definition = ptr(location, 0xc8)?;
        // Terrain comes from the province definition and class contribution from
        // the ship. Both sources are shared until the callback returns, including
        // the complete ocean fallback scan.
        let terrain = match memo.terrain {
            Some((source, terrain)) if source == definition => terrain,
            _ => {
                let terrain = sources::terrain(memory, base, definition)?;
                memo.terrain = Some((definition, terrain));
                terrain
            }
        };
        let class = field(memory, ship, CLASS_DATABASE_ID)?;
        let contribution = match memo.terrain_class {
            Some((source, selector, value)) if source == terrain && selector == class => value,
            _ => {
                let (modifier, _) = sources::terrain_class(memory, terrain, class)?;
                let value = values(add(modifier, 0x10)?)?;
                memo.terrain_class = Some((terrain, class, value));
                value
            }
        };
        accumulate(&mut total, contribution);
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
        let weather = memo.weather.as_ref().unwrap();
        let weather_scale = factor
            .wrapping_sub(100000)
            .wrapping_add(weather.country_adjustment);
        accumulate(
            &mut total,
            weather
                .region
                .map(|v| v.wrapping_mul(weather_scale) / 100000),
        );
        accumulate(&mut total, weather.location_and_task);
    }
    let fleet = ptr(task, ctask_force::FLEET)?;
    // Resolve the current task's fleet leader before reuse. The base fleet-
    // leader contribution has no ship operand.
    let (leader, contribution) = match memo.fleet {
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
    if let Some(leader) = leader {
        accumulate(&mut total, contribution);
        let subunit = field(memory, ship, 0x60c)?;
        let contribution = match memo.leader_class {
            Some((source, selector, value)) if source == leader && selector == subunit => value,
            _ => {
                let class = sources::selected_class(memory, leader, subunit, true)?;
                let value = values(add(class, 0x10)?)?;
                memo.leader_class = Some((leader, subunit, value));
                value
            }
        };
        accumulate(&mut total, contribution);
    }
    let criticals = array(ship, 0x918, 16)?;
    let mut critical = [0i64; 4];
    for index in 0..criticals.count {
        let entry = criticals
            .element(index, 16)
            .ok_or("ship critical address overflow")?;
        let (count, pointer) = organisation_pair::<4, 8>(memory, (entry, 8), (entry, 0))?;
        let count = i64::from(i32::from_le_bytes(count));
        let v = values(add(u64::from_le_bytes(pointer) as usize, 0x98)?)?;
        accumulate(
            &mut critical,
            v.map(|n| n.wrapping_mul(count).wrapping_mul(100000) / 100000),
        );
    }
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
                let Some(character) = crate::creference_registry::resolve(
                    memory,
                    base,
                    field(memory, influence, 8)?,
                )?
                else {
                    continue;
                };
                auth(character, crate::ccharacter::VTABLE_RVA)?;
                let direct = ptr(character, crate::ccharacter::UNIT_LEADER)?;
                let leader = if direct != 0 {
                    Some(direct)
                } else {
                    crate::creference_registry::resolve(
                        memory,
                        base,
                        field(memory, character, 0xc8)?,
                    )?
                };
                let Some(leader) = leader else { continue };
                if field::<i32>(memory, leader, crate::cunit_leader::CATEGORY)? != 2 {
                    continue;
                }
                auth(leader, crate::cnavy_leader::VTABLE_RVA)?;
                let mut v = [0i64; 4];
                for offset in [0xf70, 0xf80, 0xfa0, 0xf90] {
                    accumulate(&mut v, values(add(ptr(leader, offset)?, 0xe0)?)?);
                }
                for n in 0..4 {
                    support[n] = support[n].max(v[n]);
                }
            }
            memo.support = Some((task, support));
            support
        }
    };
    let critical_scale = total[3]
        .wrapping_add(critical[3])
        .wrapping_add(support[3])
        .wrapping_add(100000);
    // Native truncates each critical entry before summing; distributing division changes results.
    for index in 0..criticals.count {
        let entry = criticals
            .element(index, 16)
            .ok_or("ship critical address overflow")?;
        let (count, pointer) = organisation_pair::<4, 8>(memory, (entry, 8), (entry, 0))?;
        let count = i64::from(i32::from_le_bytes(count));
        let v = values(add(u64::from_le_bytes(pointer) as usize, 0x98)?)?;
        for n in 0..3 {
            total[n] = total[n].wrapping_add(
                (v[n].wrapping_mul(count).wrapping_mul(100000) / 100000)
                    .wrapping_mul(critical_scale)
                    / 100000,
            );
        }
    }
    for n in 0..3 {
        total[n] = total[n].wrapping_add(support[n]);
    }
    let maximum = organisation_maximum(
        field(memory, ship, BASE_ORGANISATION)?,
        [total[0], total[1], total[2]],
    );
    Ok(OrganisationObservation {
        maximum,
        handle: u64::from_le_bytes(handle),
        task,
        fleet,
        leader,
        criticals,
        critical_scale,
    })
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
    for (value, index) in values.iter_mut().zip([0, 1, 3, 2]) {
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
        if raw.present {
            *value = crate::modifier_definition::effective_memo(
                memory,
                base,
                key,
                raw.raw,
                definitions,
            )?;
        }
    }
    Ok(values)
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
