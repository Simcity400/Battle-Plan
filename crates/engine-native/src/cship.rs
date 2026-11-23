
use crate::{
    Memory, cequipment_type, cequipment_variant, cequipment_variant_pool, cname_group_member,
    ctask_force, cunit, msvc_string::Descriptor, read, recent::Recent,
};
use engine_sites::Rva;

mod air_capacity;
mod naming;
pub use naming::{NamingOperands, naming};
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

pub const VTABLE_RVA: Rva = Rva(0x029d_4888);
pub const HANDLE: usize = 0x08;
pub const HANDLE_TYPE: u32 = 51;
pub const ACQUIRED_MANPOWER: usize = 0x808;
pub const SUNK_CONVOYS: usize = 0x3c;
pub const EQUIPMENT_POOL: usize = 0x40;
pub const CLASS_DATABASE_ID: usize = 0x88;
pub const SUBUNIT_DEFINITION: usize = 0x80;
pub const SUBUNIT_INDEX: usize = SUBUNIT_DEFINITION + crate::csub_unit_definition::DATABASE_INDEX;
pub const DEFINITION: usize = 0x98;
pub const CATEGORY: usize = 0xe0;
pub const FALLBACK_SPRITE: usize = 0xe8;
pub const FORMATION_FLAGS: usize = 0x628;
pub const DEFINITION_VALID: usize = 0x90;
pub const MAXIMUM_STRENGTH: usize = 0x308;
pub const BASE_ORGANISATION: usize = 0x300;
pub const CARRIER_AIR_BASE: usize = 0x730;
pub const ANTI_AIR_BASE: usize = 0x218;
pub const MODEL_NUMBER: usize = 0x810;
pub const NAME_MEMBER: usize = 0x818;
pub const MODEL_TEXT_CAPACITY: usize = 256;
pub const STRENGTH: usize = 0x6f8;
pub const ORGANISATION: usize = 0x700;
pub const EXPERIENCE: usize = 0x708;
pub const TASK_FORCE: usize = 0x728;
pub const SIZE: usize = 0x938;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00c8_65b0);
pub use engine_native_memory::save_allocation_counters::NEXT_ID_RVA;
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
    let expected =
        engine_sites::at(base, VTABLE_RVA).ok_or("ship cached name module address overflow")?;
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
    pub id: u64,
    pub task_force: u64,
    pub strength: i64,
    pub organisation: i64,
    pub experience: i64,
    pub category: u32,
    pub sunk_convoys: i32,
    pub class_id: i32,
    pub acquired_manpower: Option<i32>,
    pub required_manpower: Option<i32>,
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
    pub has_criticals: Option<u32>,
    pub air_capacity_kind: Option<u32>,
    pub positive_equipment_flags: u64,
    pub subunit_index: Option<i32>,
    pub hull_variant: Option<HullVariantInputs>,
    pub naval_power: Option<[i64; 5]>,
}

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

#[derive(Default)]
pub struct ModelMemo {
    hulls: crate::combat_sources::Map<HullKey, ModelHull>,
    hull_variant: hull_variant::Memo,
}

impl ModelMemo {
    pub fn reset(&mut self) {
        self.hulls.clear();
        self.hull_variant.reset();
    }

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
        self.hulls.get(&key)
    }

    fn remember_hull(&mut self, hull: ModelHull) {
        if crate::allocation::reserve_map(&mut self.hulls, 1).is_ok() {
            self.hulls.insert(hull.key, hull);
        }
    }
}

type HullKey = (usize, usize, [u8; 4]);

struct ModelHull {
    key: HullKey,
    sprite: ModelText,
    native_null: bool,
}

pub fn read_model(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    source: &Row,
    memo: &mut ModelMemo,
) -> Result<ModelRow, &'static str> {
    read_model_with_selection(memory, base, pointer, source, memo).map(|(row, _)| row)
}

pub struct SelectedModelVariant {
    pointer: usize,
    handle: u64,
    definition: usize,
    token: [u8; 4],
}

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
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("ship model module overflow")?;
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
        != engine_sites::at(base, ctask_force::VTABLE_RVA).ok_or("ship model module overflow")?
            as u64
        || u64::from_le_bytes(parent_handle) != source.task_force
        || source.task_force == 0
    {
        return Err("ship model task force differs from census");
    }
    let (selected, positive_equipment_flags) =
        cequipment_variant_pool::model_selection(memory, base, at(EQUIPMENT_POOL)?)?;
    let fallback_sprite = model_text(memory, at(FALLBACK_SPRITE)?)?;
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
            engine_sites::at(base, cequipment_variant::VTABLE_RVA)
                .ok_or("ship model module overflow")?,
            identity,
        )?;
        let definition = usize::try_from(u64::from_le_bytes(definition))
            .ok()
            .filter(|pointer| *pointer != 0)
            .ok_or("ship model equipment definition pointer invalid")?;
        if let Some(hull) = memo.hull((base, definition, token)) {
            if hull.native_null && row.variant >> 32 == 0 {
                return Err("ship model equipment definition identity mismatch");
            }
            row.hull_sprite = hull.sprite.try_copy()?;
        } else {
            let definition_identity = read::<[u8; 12]>(memory, definition)
                .ok_or("ship model equipment definition identity unreadable")?;
            let ordinary = u64::from_le_bytes(definition_identity[..8].try_into().unwrap())
                == engine_sites::at(base, cequipment_type::VTABLE_RVA)
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
    let expected = engine_sites::at(base, crate::csub_unit_definition::VTABLE_RVA)
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

fn model_text(memory: &impl Memory, address: usize) -> Result<ModelText, &'static str> {
    let descriptor = Descriptor::read(memory, address)?;
    if descriptor.len > MODEL_TEXT_CAPACITY {
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
        != engine_sites::at(base, crate::cair_base::VTABLE_RVA).map(|n| n as u64)
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
pub(crate) const ROW_STRIDE: usize = 64;
pub(crate) const ROW_LAYOUT: crate::memory::RecordLayout<'static> =
    crate::memory::RecordLayout::new(&ROW_FIELDS, ROW_STRIDE).expect("valid ship row layout");

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
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
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
        if read::<u64>(memory, pointer) != Some(expected as u64) {
            return Err("ship vtable mismatch");
        }
        return Err("ship address overflow");
    };
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

#[derive(Default)]
pub struct OrganisationMemo {
    last: Option<(usize, [i64; 4])>,
    owner: Option<(usize, u32, usize)>,
    country: Option<(usize, [i64; 4])>,
    pride: Recent<usize, [u8; 8], RECENT>,
    support: Option<(usize, [i64; 4])>,
    influences: Recent<u64, Option<[i64; 4]>, 16>,
    terrain: Recent<usize, usize, RECENT>,
    location: Option<(usize, usize)>,
    ocean: Option<usize>,
    terrain_class: Recent<(usize, u32), [i64; 4], RECENT>,
    fleet: Option<(usize, Option<usize>, [i64; 4])>,
    leader_class: Recent<(usize, u32), [i64; 4], RECENT>,
    doctrine_keys: crate::naval_ship_modifier_inputs::DoctrineKeys,
    weather: Option<OrganisationWeather>,
    definitions: crate::modifier_definition::DefinitionMemo,
    own: Option<(usize, OrganisationOperands)>,
    modifier_guards: [Option<u16>; 1 << ORGANISATION_KEYS.len()],
}

const RECENT: usize = 8;

impl OrganisationMemo {
    pub fn organisation_operands(&self, ship: usize) -> Option<OrganisationOperands> {
        self.own
            .filter(|&(pointer, _)| pointer == ship)
            .map(|(_, operands)| operands)
    }

    pub fn hold_doctrine(&mut self, task: usize, contribution: [i64; 4]) {
        self.last = Some((task, contribution));
    }

    pub fn hold_country(&mut self, country: usize, terms: [i64; 4]) {
        self.country = Some((country, terms));
    }

    pub fn country_terms(&self) -> Option<(usize, [i64; 4])> {
        self.country
    }

    pub fn doctrine(&self, task: usize) -> Option<[i64; 4]> {
        self.last
            .filter(|&(observed, _)| observed == task)
            .map(|(_, contribution)| contribution)
    }

    pub fn hold_weather(&mut self, weather: OrganisationWeather) {
        self.weather = Some(weather);
    }

    pub fn weather(&self) -> Option<OrganisationWeather> {
        self.weather
    }

    pub fn hold_location(&mut self, location: usize, definition: usize, terrain: usize) {
        self.location = Some((location, definition));
        self.terrain.insert(definition, terrain);
    }

    pub fn location_source(&self, location: usize) -> Option<(usize, usize)> {
        let (_, definition) = self.location.filter(|&(held, _)| held == location)?;
        Some((definition, self.terrain.get(&definition)?))
    }

    pub fn hold_fleet(&mut self, fleet: usize, leader: Option<usize>, terms: [i64; 4]) {
        self.fleet = Some((fleet, leader, terms));
    }

    pub fn fleet_source(&self, fleet: usize) -> Option<(Option<usize>, [i64; 4])> {
        self.fleet
            .filter(|&(held, _, _)| held == fleet)
            .map(|(_, leader, terms)| (leader, terms))
    }

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

    pub fn country(&self) -> Option<usize> {
        self.owner.map(|(_, _, country)| country)
    }
}

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
    pub fn has_criticals(&self) -> bool {
        self.criticals.count != 0
    }

    pub fn modifier_guard(&self) -> u16 {
        self.modifier_guard
    }
}

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
    pub fn leader(&self) -> Option<usize> {
        self.fleet.1
    }

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
    pub fn task(&self) -> usize {
        self.sources[1]
    }

    pub fn region(&self) -> usize {
        self.region_at
    }

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
    if field::<u64>(memory, weather, 0)? as usize
        != organisation_global(base, crate::cweather_manager::VTABLE_RVA)?
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

fn organisation_global(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key)
        .filter(|_| base != 0)
        .ok_or("ship organisation pointer overflow")
}

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
        if ptr(p, 0)? != organisation_global(base, rva)? {
            Err("ship organisation source identity mismatch")
        } else {
            Ok(())
        }
    };
    let task_at = task.pointer;
    auth(task_at, ctask_force::VTABLE_RVA)?;
    let (game, country_id) = organisation_pair::<8, 4>(
        memory,
        (
            organisation_global(base, crate::cgame_state::SINGLETON_RVA)?,
            0,
        ),
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
        let definition = match memo.location {
            Some((held, definition)) if held == location => definition,
            _ => {
                auth(location, crate::cprovince::VTABLE_RVA)?;
                let definition = ptr(location, 0xc8)?;
                memo.location = Some((location, definition));
                definition
            }
        };
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
        if ptr(p, 0)? != organisation_global(base, rva)? {
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

pub fn read_maximum_organisation_memo(
    memory: &impl Memory,
    base: usize,
    ship: usize,
    memo: &mut OrganisationMemo,
) -> Result<i64, &'static str> {
    memo.own = None;
    read_organisation_observation(memory, base, ship, memo)
}

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
        if ptr(p, 0)? != organisation_global(base, rva)? {
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
        (
            organisation_global(base, crate::cgame_state::SINGLETON_RVA)?,
            0,
        ),
        (task, cunit::ROSTER_OWNER),
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
        let root = ptr(organisation_global(base, Rva(0x033d_e590))?, 0)?;
        values(add(ptr(root, 0xb0)?, 0x10)?)?
    } else {
        [0; 4]
    };
    let own = organisation_modifier_observation(memory, base, add(ship, 0x750)?, definitions)?;
    let experience = ptr(ship, 0x8f8)?;
    let medals =
        organisation_modifier_observation(memory, base, add(experience, 0x1b0)?, definitions)?;
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
        let definition = match memo.location {
            Some((held, definition)) if held == location => definition,
            _ => {
                auth(location, crate::cprovince::VTABLE_RVA)?;
                let definition = ptr(location, 0xc8)?;
                memo.location = Some((location, definition));
                definition
            }
        };
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
    operands.base_organisation = field(memory, ship, BASE_ORGANISATION)?;
    let maximum = organisation_maximum(operands.base_organisation, modifiers);
    memo.own = Some((ship, operands));
    Ok(maximum)
}

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
        let root = field::<u64>(memory, organisation_global(base, Rva(0x033d_e590))?, 0)? as usize;
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
        word(&mut bytes, SHIP, VTABLE_RVA.0 as u64);
        word(&mut bytes, SHIP + HANDLE, FULL_HANDLE);
        word(&mut bytes, SHIP + NAME_MEMBER, MEMBER as u64);
        word(&mut bytes, MEMBER, cname_group_member::VTABLE_RVA.0 as u64);
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
                bytes: crate::walk::Buffer::Growing(&mut arena),
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
            bytes: crate::walk::Buffer::Count,
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
            bytes: crate::walk::Buffer::Count,
            len: 0,
            required: 0,
        };
        assert!(cached_name(&tracked(&bytes, None), 0, SHIP, FULL_HANDLE, &mut names).is_err());
        assert_eq!(names.required, 0);
        bytes[HEAP + text.len()] = 0;
        let mut arena = vec![0; text.len()];
        let mut names = cname_group_member::Names {
            bytes: crate::walk::Buffer::Growing(&mut arena),
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
