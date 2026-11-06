//! CEquipmentVariant identity and fields shared by equipment, production and air.
//!
//! The native stat writers preserve signed 64-bit cache values. The supplied
//! published campaign census comes from the game's dense saved equipment array.
//! A truncated census is not a complete source for its dependent leaf tables.

mod design;
pub mod mutations;
pub mod source_mutations;
pub use design::*;

use crate::{
    Memory, cequipment_type, cgame_state,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    msvc_string, read, read_slice,
    walk::Outcome,
};

/// CEquipmentVariant vtable.
pub const VTABLE_RVA: usize = 0x029c_e250;
/// CEquipmentVariant reference type.
pub const REFERENCE_TYPE: u32 = 70;
/// CEquipmentVariant u32 type word.
pub const TYPE: usize = 0x08;
/// CEquipmentVariant u32 id.
pub const ID: usize = 0x0c;
/// CEquipmentVariant u32 equipment token.
pub const EQUIPMENT_TOKEN: usize = 0x18;
/// CEquipmentVariant creator-country link u32.
pub const CREATOR_LINK: usize = 0x1c;
/// CEquipmentVariant origin-country link u32.
pub const ORIGIN_LINK: usize = 0x20;
/// CEquipmentVariant raw append-variant-name byte.
#[cfg(any(test, feature = "test-support"))]
pub const APPEND_VARIANT_NAME: usize = 0x24;
/// CEquipmentVariant display-name MSVC string.
pub const DISPLAY_NAME: usize = 0x28;
/// CEquipmentVariant variant-name MSVC string.
pub const VARIANT_NAME: usize = 0x48;
/// CEquipmentVariant embedded CEquipmentUpgradesInstance.
pub const UPGRADES: usize = 0x68;
/// Embedded variant module collection, with its data pointer and count; its vtable remains
/// unproven.
pub const MODULES: usize = 0xb0;
/// Equipment-variant map-range stat in its ordered stat array.
pub const MAP_RANGE_STAT: usize = 0x270;
/// The variant stat after the map range, the second retained variant-stat lane.
#[cfg(any(test, feature = "test-support"))]
pub const STAT_2F: usize = 0x278;
/// Variant attack cache, signed i64; live consistency and save agreement remain unverified.
#[cfg(any(test, feature = "test-support"))]
pub const AIR_ATTACK_CACHE_CANDIDATE: usize = 0x280;
/// Variant agility cache, signed i64; live consistency and save agreement remain unverified.
#[cfg(any(test, feature = "test-support"))]
pub const AGILITY_CACHE_CANDIDATE: usize = 0x288;
/// Variant maximum-speed cache, signed i64; live consistency and save agreement remain unverified.
pub const MAXIMUM_SPEED_CACHE_CANDIDATE: usize = 0x2f0;
/// Variant reliability cache, signed i64; live consistency and save agreement remain unverified.
pub const RELIABILITY_CACHE_CANDIDATE: usize = 0x308;
/// Variant defence cache, signed i64; live consistency and save agreement remain unverified.
pub const AIR_DEFENSE_CACHE_CANDIDATE: usize = 0x318;
/// Variant thrust cache, signed i64; live consistency and save agreement remain unverified.
#[cfg(any(test, feature = "test-support"))]
pub const THRUST_CACHE_CANDIDATE: usize = 0x320;
/// Variant fuel cache, signed i64; live consistency and save agreement remain unverified.
#[cfg(any(test, feature = "test-support"))]
pub const FUEL_CONSUMPTION_CACHE_CANDIDATE: usize = 0x328;
/// Variant build-cost cache, signed i64; live consistency and save agreement remain unverified.
pub const BUILD_COST_CACHE_CANDIDATE: usize = 0x368;
/// CEquipmentVariant mission-group array data.
pub const MISSION_GROUP_DATA: usize = 0x370;
/// CEquipmentVariant CEquipmentType definition pointer.
pub const DEFINITION: usize = 0x3f0;
/// CEquipmentVariant classifier flags u64.
pub const CLASSIFIER_FLAGS: usize = 0x408;
/// CEquipmentVariant convoy classifier bit.
pub const CONVOY_CLASSIFIER_MASK: u64 = 1;
/// CEquipmentVariant capital-ship classifier bit.
pub const CAPITAL_SHIP_CLASSIFIER_MASK: u64 = 1 << 6;
/// CEquipmentVariant floating-harbour classifier bit.
pub const FLOATING_HARBOR_CLASSIFIER_MASK: u64 = 1 << 9;
/// CEquipmentVariant mission mask u32.
pub const MISSION_MASK: usize = 0x410;
/// Raw version byte. Construction clears it, loading restores it,
/// derivative creation increments the base byte and cloning copies it.
/// Serialization zero-extends nonzero values.
pub const VERSION: usize = 0x41c;
/// CEquipmentVariant obsolete byte.
pub const OBSOLETE: usize = 0x424;
/// CEquipmentVariant explicit entity MSVC string.
pub const EXPLICIT_ENTITY: usize = 0x430;
/// Variant icon entry: 0x30 bytes with its texture pointer at `ICON_ENTRY_RECORDED`. A new entry is
/// inactive, kind 1 and empty; loaded or constructed overrides are active. Binding tries this entry
/// before defaults. Serialization writes an active entry string without its kind.
pub const ICON_OVERRIDE_ACTIVE: usize = 0x450;
/// CEquipmentVariant icon entry u32 kind: 0 a GFX sprite name, 1 a texture path.
pub const ICON_OVERRIDE_KIND: usize = 0x454;
/// Variant icon MSVC string contains a sprite name or path. The auto-design
/// sentinel with kind 1 selects an equipment graphic-pool lookup instead.
pub const ICON_OVERRIDE_TOKEN: usize = 0x458;
/// Recorded icon bytes: active flag, kind and string header.
///
/// The remaining rule uses content and recorded country values; see
/// `replay_data::rows::VariantDefinitionRow`. An active valid override binds first.
/// Otherwise type, origin or creator country, classifier and size select the
/// content name search. Dynamic creators use their original tag.
/// Auto-design resolves graphic pools using type, ruling ideology and 3D culture.
/// Type and active archetype names are tried in order, followed by enabling
/// technology names with country, 2D culture and unprefixed variants; family
/// links recurse. Pools come from country, faction, continent and default
/// contexts. A stable descending weight sort precedes reverse icon iteration
/// within each pool, retaining the pool's content eligibility rules.
pub const ICON_ENTRY_RECORDED: usize = 0x28;
/// Equipment-variant constructor initializes the vtable and mission storage.
pub const CONSTRUCTOR_RVA: usize = 0x00c2_6060;

/// Authenticates the variant and preserves its native low-type/high-id reference.
pub fn identity(memory: &impl Memory, base: usize, pointer: usize) -> Result<u64, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("variant module address overflow")?;
    let bytes = read::<[u8; 16]>(memory, pointer).ok_or("variant identity unreadable")?;
    identity_header(expected, bytes)
}

/// Decode an identity already gathered with the rest of its batch. The caller
/// supplies the relocated vtable so a batch computes it once.
pub fn identity_from_header(expected: usize, bytes: [u8; 16]) -> Result<u64, &'static str> {
    identity_header(expected, bytes)
}

fn identity_header(expected: usize, bytes: [u8; 16]) -> Result<u64, &'static str> {
    let vtable = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let handle = u64::from_le_bytes(bytes[8..].try_into().unwrap());
    if vtable != expected as u64 || handle as u32 != REFERENCE_TYPE {
        return Err("variant vtable or reference type mismatch");
    }
    Ok(handle)
}

/// Authenticated identity and callback-local address, never a serialized pointer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RegistryRow {
    pub id: u64,
    address: usize,
}

impl RegistryRow {
    /// Authenticate a single native reference for a bounded dependent read.
    pub fn read(memory: &impl Memory, base: usize, pointer: usize) -> Result<Self, &'static str> {
        if pointer == 0 {
            return Err("variant pointer is null");
        }
        Ok(Self {
            id: identity(memory, base, pointer)?,
            address: pointer,
        })
    }
    /// Use only while reading dependent tables within the same callback.
    pub fn pointer(self) -> usize {
        self.address
    }
}

/// Copies the published, saved campaign variants, including unused variants.
/// Every native slot is a row, starting at zero; there is no sentinel or hole.
/// Pointer spans and retained identity headers use bounded batches. Storage
/// grows on the acquisition worker only for copied identity batches.
/// Native order and duplicate identities are preserved; spool conversion must
/// reject duplicate row keys instead of silently deduplicating the census.
/// Publication scheduling, consistency and cost remain separate acceptance
/// requirements. A successful guarded read does not prove update ordering.
pub fn walk_registry(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<RegistryRow>,
) -> Outcome {
    let result = (|| {
        let singleton = base
            .checked_add(cgame_state::SINGLETON_RVA)
            .ok_or("variant registry module address overflow")?;
        if game_state == 0 || read::<u64>(memory, singleton) != Some(game_state as u64) {
            return Err("variant registry game state unreadable or changed");
        }
        let array = Array::read(memory, at(game_state, cgame_state::EQUIPMENT_VARIANTS)?, 8)
            .ok_or("variant registry array unreadable or invalid")?;
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("variant module address overflow")?;
        const BATCH: usize = 64;
        let mut pointer_bytes = [0_u8; BATCH * 8];
        let mut headers = [[0_u8; 16]; BATCH];
        let mut first = 0_u32;
        let mut len = 0;
        while first < array.count {
            let count = ((array.count - first) as usize).min(BATCH);
            let source = array
                .element(first, 8)
                .ok_or("variant registry array address overflow")?;
            if !read_slice(memory, source, &mut pointer_bytes[..count * 8]) {
                return Err("variant registry pointers unreadable");
            }
            let mut addresses = [0_usize; BATCH];
            for (address, bytes) in addresses
                .iter_mut()
                .zip(pointer_bytes.chunks_exact(8))
                .take(count)
            {
                *address = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                    .ok_or("variant registry pointer null or invalid")?;
            }
            let retained = count;
            if retained != 0 {
                let mut fields = headers.iter_mut().enumerate();
                let copied = memory.copy_scattered(
                    addresses,
                    &mut std::array::from_fn(|_| {
                        let (slot, bytes) = fields.next().unwrap();
                        &mut bytes[..if slot < retained { 16 } else { 0 }]
                    }),
                );
                let required = len + copied.min(retained);
                if required > output.len() {
                    crate::allocation::reserve(output, required - output.len())
                        .map_err(|_| "variant registry allocation failed")?;
                    output.resize(required, RegistryRow::default());
                }
                for slot in 0..copied.min(retained) {
                    output[len] = RegistryRow {
                        id: identity_header(expected, headers[slot])?,
                        address: addresses[slot],
                    };
                    len += 1;
                }
                if copied < retained {
                    return Err("variant identity unreadable");
                }
            }
            first += count as u32;
        }
        Ok(Outcome::Rows { len })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// Runtime identity fields; country values remain the engine's reference links.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub equipment_token: u32,
    /// Definition archetype token, selected through its archetype link;
    /// the pool total is keyed by this token.
    pub archetype_token: u32,
    pub creator_link: u32,
    pub origin_link: u32,
    pub append_variant_name: u8,
    pub obsolete: u8,
    pub classifier_flags: u64,
    pub mission_mask: u32,
}

/// Exact bytes in the caller's arena. Empty strings remain present empty strings.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TextSpan {
    pub start: usize,
    pub len: usize,
}

/// The equipment type is named by `identity.equipment_token`; its text is
/// content, so the name string is not copied per variant.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DefinitionRow<D = VariantDesignState> {
    pub identity: Row,
    /// Authenticated pass-local type address, never a replay identity.
    pub archetype_pointer: usize,
    /// Sample-local organisation resolution already used for save admission.
    /// Only the current native visit may dereference it; never a replay identity.
    pub organisation_pointer: Option<usize>,
    /// Sample-local direct-source bindings; never serialized. None for a carried deep value.
    pub source_links: Option<DesignSourceLinks>,
    pub display_name: TextSpan,
    pub variant_name: TextSpan,
    /// The designer's saved model choice; empty when unset.
    pub explicit_entity: TextSpan,
    /// The icon entry's raw active byte, kind and string (see
    /// [`ICON_OVERRIDE_ACTIVE`]); inactive entries keep their bytes too.
    pub icon_active: u8,
    pub icon_kind: u32,
    pub icon: TextSpan,
    pub version: u8,
    pub design: D,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextOutcome {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

/// Reads each supplied variant without caching definitions across hours.
/// Supply the complete registry census for published campaign coverage; a
/// truncated or unavailable source cannot become a complete leaf table.
/// Keep duplicate handles for spool rejection, including conflicts between
/// different pointers. The same prepared census feeds stats and children.
pub fn walk_definitions(
    memory: &impl Memory,
    base: usize,
    variants: &[RegistryRow],
    output: &mut Vec<DefinitionRow>,
    bytes: &mut Vec<u8>,
) -> TextOutcome {
    walk_definitions_with(memory, base, variants, output, bytes, Ok)
}

/// The same native visit with worker-owned applied-source interning. Mapping
/// runs only after the design's complete native inputs have been read.
pub fn walk_definitions_with<D: Default>(
    memory: &impl Memory,
    base: usize,
    variants: &[RegistryRow],
    output: &mut Vec<DefinitionRow<D>>,
    bytes: &mut Vec<u8>,
    map: impl FnMut(VariantDesignState) -> Result<D, &'static str>,
) -> TextOutcome {
    walk_definitions_carried(memory, base, variants, output, bytes, |_| Ok(None), map)
}

/// A caller with a complete writer certificate may carry the added deep state.
/// Prefix authentication and text reads retain their ordinary census contract.
/// Only misses request deep spans; carried values never authorize native reads.
pub fn walk_definitions_carried<D: Default>(
    memory: &impl Memory,
    base: usize,
    variants: &[RegistryRow],
    output: &mut Vec<DefinitionRow<D>>,
    bytes: &mut Vec<u8>,
    mut carry: impl FnMut(&RegistryRow) -> Result<Option<D>, &'static str>,
    mut map: impl FnMut(VariantDesignState) -> Result<D, &'static str>,
) -> TextOutcome {
    let result = (|| {
        let mut len = 0;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        if variants.len() > output.len() {
            crate::allocation::reserve(output, variants.len() - output.len())
                .map_err(|_| "variant definition allocation failed")?;
            output.resize_with(variants.len(), DefinitionRow::default);
        }
        const BATCH: usize = 64;
        const PREFIX_WIDTH: usize = 131 + ICON_ENTRY_RECORDED;
        const STRIDE: usize = PREFIX_WIDTH + design::DESIGN_WIDTH;
        const PREFIX_FIELDS: [RecordField; 8] = [
            RecordField {
                source_offset: EQUIPMENT_TOKEN,
                destination_offset: 0,
                len: 13,
            },
            RecordField {
                source_offset: DEFINITION,
                destination_offset: 13,
                len: 8,
            },
            RecordField {
                source_offset: OBSOLETE,
                destination_offset: 21,
                len: 1,
            },
            RecordField {
                source_offset: CLASSIFIER_FLAGS,
                destination_offset: 22,
                len: 12,
            },
            RecordField {
                source_offset: DISPLAY_NAME,
                destination_offset: 34,
                len: 64,
            },
            RecordField {
                source_offset: EXPLICIT_ENTITY,
                destination_offset: 98,
                len: 32,
            },
            RecordField {
                source_offset: ICON_OVERRIDE_ACTIVE,
                destination_offset: 130,
                len: ICON_ENTRY_RECORDED,
            },
            RecordField {
                source_offset: VERSION,
                destination_offset: 130 + ICON_ENTRY_RECORDED,
                len: 1,
            },
        ];
        const FIELDS: [RecordField; PREFIX_FIELDS.len() + design::DESIGN_FIELDS.len()] = {
            let mut result = [RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 0,
            }; PREFIX_FIELDS.len() + design::DESIGN_FIELDS.len()];
            let mut i = 0;
            while i < PREFIX_FIELDS.len() {
                result[i] = PREFIX_FIELDS[i];
                i += 1;
            }
            let mut j = 0;
            while j < design::DESIGN_FIELDS.len() {
                let field = design::DESIGN_FIELDS[j];
                result[i] = RecordField {
                    source_offset: field.source_offset,
                    destination_offset: PREFIX_WIDTH + field.destination_offset,
                    len: field.len,
                };
                i += 1;
                j += 1;
            }
            result
        };
        const EQUIPMENT_FIELDS: [RecordField; 2] = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 12,
            },
            RecordField {
                source_offset: cequipment_type::ARCHETYPE,
                destination_offset: 12,
                len: 8,
            },
        ];
        const ARCHETYPE_FIELDS: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 12,
        }];
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&FIELDS, STRIDE).expect("valid record layout");
        const PREFIX_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&PREFIX_FIELDS, STRIDE).expect("valid prefix layout");
        const DESIGN_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&design::DESIGN_FIELDS, design::DESIGN_WIDTH)
                .expect("valid design layout");
        const EQUIPMENT_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&EQUIPMENT_FIELDS, 20).expect("valid record layout");
        const ARCHETYPE_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&ARCHETYPE_FIELDS, 12).expect("valid record layout");
        let expected = base
            .checked_add(cequipment_type::VTABLE_RVA)
            .ok_or("equipment module address overflow")?;
        let null_expected = base
            .checked_add(cequipment_type::NULL_VTABLE_RVA)
            .ok_or("equipment module address overflow")?;
        let equipment_class = |header: &[u8]| {
            let vtable = u64::from_le_bytes(header[..8].try_into().unwrap());
            vtable == expected as u64 || vtable == null_expected as u64
        };
        let mut pointers = [0; BATCH];
        let mut definitions = [0; BATCH];
        let mut fields = [0; BATCH * STRIDE];
        let mut equipment = [0; BATCH * 20];
        let mut archetypes = [0; BATCH];
        let mut archetype_bytes = [0; BATCH * 12];
        let mut references = crate::creference_registry::ScopedResolver::new(base);
        for sources in variants.chunks(BATCH) {
            let mut carried: [Option<D>; BATCH] = std::array::from_fn(|_| None);
            let mut misses = [0; BATCH];
            let mut miss_count = 0;
            for (index, source) in sources.iter().enumerate() {
                carried[index] = carry(source)?;
                if carried[index].is_none() {
                    misses[miss_count] = index;
                    miss_count += 1;
                }
            }
            for (pointer, source) in pointers.iter_mut().zip(sources) {
                *pointer = source.pointer();
            }
            let layout = if miss_count == sources.len() {
                &LAYOUT
            } else {
                &PREFIX_LAYOUT
            };
            let copied = memory.copy_records(
                RecordSources::Pointers(&pointers[..sources.len()]),
                layout,
                &mut fields,
            );
            let field_count = if miss_count == sources.len() {
                FIELDS.len()
            } else {
                PREFIX_FIELDS.len()
            };
            let ready = copied / field_count;
            let mut deep_ready = ready;
            if miss_count != 0 && miss_count != sources.len() {
                let mut demanded = [0; BATCH];
                let count = misses[..miss_count].partition_point(|index| *index < ready);
                for (pointer, index) in demanded.iter_mut().zip(&misses[..count]) {
                    *pointer = pointers[*index];
                }
                let mut deep = [0; BATCH * design::DESIGN_WIDTH];
                let copied = memory.copy_records(
                    RecordSources::Pointers(&demanded[..count]),
                    &DESIGN_LAYOUT,
                    &mut deep,
                );
                let complete = copied / design::DESIGN_FIELDS.len();
                for (ordinal, index) in misses[..complete].iter().enumerate() {
                    fields[*index * STRIDE + PREFIX_WIDTH..(*index + 1) * STRIDE].copy_from_slice(
                        &deep[ordinal * design::DESIGN_WIDTH..(ordinal + 1) * design::DESIGN_WIDTH],
                    );
                }
                if complete < count {
                    deep_ready = misses[complete];
                }
            }
            for (pointer, field) in definitions
                .iter_mut()
                .zip(fields.chunks_exact(STRIDE))
                .take(ready)
            {
                *pointer = usize::try_from(u64::from_le_bytes(field[13..21].try_into().unwrap()))
                    .unwrap_or(0);
            }
            let equipment_copied = memory.copy_records(
                RecordSources::Pointers(&definitions[..ready]),
                &EQUIPMENT_LAYOUT,
                &mut equipment,
            );
            let equipment_ready = equipment_copied / EQUIPMENT_FIELDS.len();
            for (pointer, row) in archetypes
                .iter_mut()
                .zip(equipment.chunks_exact(20))
                .take(equipment_ready)
            {
                *pointer = usize::try_from(u64::from_le_bytes(row[12..20].try_into().unwrap()))
                    .unwrap_or(0);
            }
            let archetype_copied = memory.copy_records(
                RecordSources::Pointers(&archetypes[..equipment_ready]),
                &ARCHETYPE_LAYOUT,
                &mut archetype_bytes,
            );
            for (index, variant) in sources.iter().enumerate() {
                if index >= ready {
                    return Err(match copied % field_count {
                        0 => "variant identity fields unreadable",
                        1 => "variant equipment definition unreadable or null",
                        2 => "variant obsolete byte unreadable",
                        3 => "variant classifier flags or mission mask unreadable",
                        4 => "variant string headers unreadable",
                        5 => "variant explicit entity header unreadable",
                        6 => "variant icon entry unreadable",
                        7 => "variant version byte unreadable",
                        _ => "variant design fields unreadable",
                    });
                }
                if index >= deep_ready {
                    return Err("variant design fields unreadable");
                }
                if definitions[index] == 0 {
                    return Err("variant equipment definition unreadable or null");
                }
                if equipment_copied < (index + 1) * EQUIPMENT_FIELDS.len() {
                    return Err("equipment definition identity unreadable");
                }
                if archetypes[index] == 0 || archetype_copied <= index {
                    return Err("equipment archetype unreadable or null");
                }
                let field = &fields[index * STRIDE..(index + 1) * STRIDE];
                let definition = &equipment[index * 20..(index + 1) * 20];
                let archetype = &archetype_bytes[index * 12..(index + 1) * 12];
                let equipment_token = u32::from_le_bytes(field[..4].try_into().unwrap());
                if !equipment_class(definition)
                    || u32::from_le_bytes(definition[8..12].try_into().unwrap()) != equipment_token
                {
                    return Err("variant equipment definition identity mismatch");
                }
                if !equipment_class(archetype) {
                    return Err("variant equipment archetype identity mismatch");
                }
                let row = Row {
                    id: variant.id,
                    equipment_token,
                    archetype_token: u32::from_le_bytes(archetype[8..].try_into().unwrap()),
                    creator_link: u32::from_le_bytes(field[4..8].try_into().unwrap()),
                    origin_link: u32::from_le_bytes(field[8..12].try_into().unwrap()),
                    append_variant_name: field[12],
                    obsolete: field[21],
                    classifier_flags: u64::from_le_bytes(field[22..30].try_into().unwrap()),
                    mission_mask: u32::from_le_bytes(field[30..34].try_into().unwrap()),
                };
                let strings = [
                    Literal::from_header(field[34..66].try_into().unwrap())?,
                    Literal::from_header(field[66..98].try_into().unwrap())?,
                    Literal::from_header(field[98..130].try_into().unwrap())?,
                    Literal::from_header(field[138..170].try_into().unwrap())?,
                ];
                let wanted = strings
                    .iter()
                    .try_fold(0_usize, |n, s| n.checked_add(s.len))
                    .ok_or("variant string size overflow")?;
                required_bytes = required_bytes
                    .checked_add(wanted)
                    .ok_or("variant string size overflow")?;
                let mut spans = [TextSpan::default(); 4];
                for (span, string) in spans.iter_mut().zip(&strings) {
                    let size = string.len;
                    string.copy_growing(
                        memory,
                        bytes,
                        bytes_len,
                        "variant name allocation failed",
                    )?;
                    *span = TextSpan {
                        start: bytes_len,
                        len: size,
                    };
                    bytes_len += size;
                }
                let (design, organisation_pointer, source_links) =
                    if let Some(carried) = carried[index].take() {
                        (carried, None, None)
                    } else {
                        let (state, pointer, links) = design::read_design_prefetched(
                            memory,
                            base,
                            variant.pointer(),
                            &field[PREFIX_WIDTH..],
                            &mut references,
                        )?;
                        (map(state)?, pointer, Some(links))
                    };
                output[len] = DefinitionRow {
                    identity: row,
                    archetype_pointer: archetypes[index],
                    organisation_pointer,
                    source_links,
                    display_name: spans[0],
                    variant_name: spans[1],
                    explicit_entity: spans[2],
                    icon_active: field[130],
                    icon_kind: u32::from_le_bytes(field[134..138].try_into().unwrap()),
                    icon: spans[3],
                    version: field[130 + ICON_ENTRY_RECORDED],
                    design,
                };
                len += 1;
            }
        }
        Ok(TextOutcome {
            outcome: Outcome::Rows { len },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(text_failure)
}

/// Number of native equipment statistic slots in the authenticated executable.
/// Mods can supply any of these slots; none is selected by equipment family.
pub const STAT_COUNT: usize = 78;
/// First native signed fixed5 statistic. Slot names come from executable metadata.
pub const STATS: usize = 0x100;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StatsRow {
    pub variant: u64,
    pub values: [i64; 10],
    pub organisation: Organisation,
}

/// Frozen design authorship and bonus entries, independent of current MIO traits.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Organisation {
    pub author: u64,
    pub trait_count: i32,
    pub bonuses: Vec<OrganisationBonus>,
}
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
#[derive(replay_spool::Spooled)]
pub struct OrganisationBonus {
    pub key: u32,
    pub value_raw: i64,
}
pub const INDUSTRIAL_ORGANISATION: usize = 0x488;
pub const ORGANISATION_BONUSES: usize = 0x498;
pub const ORGANISATION_TRAIT_COUNT: usize = 0x4b0;
/// Receiver certificates are checked only in the existing daily detail walk.
pub trait OrganisationCarry {
    fn fresh(&mut self, index: usize, source: &RegistryRow) -> Result<bool, &'static str>;
    fn read(&mut self, _index: usize, _source: &RegistryRow) {}
}
impl OrganisationCarry for () {
    fn fresh(&mut self, _: usize, _: &RegistryRow) -> Result<bool, &'static str> {
        Ok(true)
    }
}
fn read_organisation(
    memory: &impl Memory,
    pointer: usize,
    row: &mut Organisation,
) -> Result<(), &'static str> {
    let mut author = [0_u8; 8];
    let mut marker = [0_u8; 4];
    let mut header = [0_u8; 16];
    if memory.copy_scattered(
        [
            at(pointer, INDUSTRIAL_ORGANISATION)?,
            at(pointer, ORGANISATION_TRAIT_COUNT)?,
            at(pointer, ORGANISATION_BONUSES)?,
        ],
        &mut [&mut author, &mut marker, &mut header],
    ) != 3
    {
        return Err("design MIO fields unreadable");
    }
    row.author = u64::from_le_bytes(author);
    row.trait_count = i32::from_le_bytes(marker);
    let array = Array::from_header(header, 16).ok_or("design MIO bonus array invalid")?;
    crate::allocation::resize_default(&mut row.bonuses, array.count as usize)
        .map_err(|_| "design MIO bonus allocation failed")?;
    for (index, bonus) in row.bonuses.iter_mut().enumerate() {
        let address = array
            .element(index as u32, 16)
            .ok_or("design MIO bonus address overflow")?;
        bonus.key = read(memory, address).ok_or("design MIO bonus key unreadable")?;
        bonus.value_raw =
            read(memory, at(address, 8)?).ok_or("design MIO bonus value unreadable")?;
    }
    Ok(())
}

/// Uses the same complete supplied variant census as `walk_definitions`. A
/// worker grows storage from that already-owned census before the first batch.
#[cfg(test)]
pub fn walk_stats(
    memory: &impl Memory,
    variants: &[RegistryRow],
    output: &mut Vec<StatsRow>,
) -> Outcome {
    walk_stats_carried(memory, variants, output, &mut ())
}

pub fn walk_stats_carried(
    memory: &impl Memory,
    variants: &[RegistryRow],
    output: &mut Vec<StatsRow>,
    carry: &mut impl OrganisationCarry,
) -> Outcome {
    let result = (|| {
        if variants.len() > output.len() {
            crate::allocation::reserve(output, variants.len() - output.len())
                .map_err(|_| "variant stat allocation failed")?;
            output.resize(variants.len(), StatsRow::default());
        }
        let mut len = 0;
        const BATCH: usize = 64;
        const FIELDS: [RecordField; 5] = [
            RecordField {
                source_offset: MAP_RANGE_STAT,
                destination_offset: 0,
                len: 32,
            },
            RecordField {
                source_offset: MAXIMUM_SPEED_CACHE_CANDIDATE,
                destination_offset: 32,
                len: 8,
            },
            RecordField {
                source_offset: RELIABILITY_CACHE_CANDIDATE,
                destination_offset: 40,
                len: 8,
            },
            RecordField {
                source_offset: AIR_DEFENSE_CACHE_CANDIDATE,
                destination_offset: 48,
                len: 24,
            },
            RecordField {
                source_offset: BUILD_COST_CACHE_CANDIDATE,
                destination_offset: 72,
                len: 8,
            },
        ];
        let mut pointers = [0; BATCH];
        let mut fields = [0; BATCH * 80];
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&FIELDS, 80).expect("valid record layout");
        for sources in variants.chunks(BATCH) {
            for (pointer, source) in pointers.iter_mut().zip(sources) {
                *pointer = source.pointer();
            }
            // One borrowed layout and guard per batch, still only the exact
            // stat bytes; no gaps, names or other table fields are touched.
            if memory.copy_records(
                RecordSources::Pointers(&pointers[..sources.len()]),
                &LAYOUT,
                &mut fields,
            ) != sources.len() * FIELDS.len()
            {
                return Err("variant stat cache unreadable");
            }
            for (source, field) in sources.iter().zip(fields.chunks_exact(80)) {
                let mut values = [0_i64; 10];
                for (value, bytes) in values.iter_mut().zip(field.chunks_exact(8)) {
                    *value = i64::from_le_bytes(bytes.try_into().unwrap());
                }
                if carry.fresh(len, source)? {
                    read_organisation(memory, source.pointer(), &mut output[len].organisation)?;
                }
                output[len].variant = source.id;
                output[len].values = values;
                carry.read(len, source);
                len += 1;
            }
        }
        Ok(Outcome::Rows { len })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

pub(crate) fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("variant address overflow")
}

pub(crate) fn text_failure(reason: &'static str) -> TextOutcome {
    TextOutcome {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    }
}

/// Already-copied MSVC header, with inline text retained locally. Heap text and
/// its terminator use one guarded copy when admitted to the caller's arena.
pub(crate) struct Literal {
    inline: [u8; 16],
    heap: Option<usize>,
    pub len: usize,
}

impl Literal {
    pub fn from_header(header: [u8; 32]) -> Result<Self, &'static str> {
        let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
        let capacity = u64::from_le_bytes(header[24..].try_into().unwrap());
        if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
            return Err("equipment string header invalid");
        }
        let len = usize::try_from(len).map_err(|_| "equipment string length overflow")?;
        let inline = header[..16].try_into().unwrap();
        let heap = if capacity == msvc_string::INLINE_CAPACITY {
            if header[len] != 0 {
                return Err("equipment string terminator unreadable or invalid");
            }
            None
        } else {
            let pointer = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
                .ok()
                .filter(|&pointer| pointer != 0)
                .ok_or("equipment string pointer null or invalid")?;
            at(pointer, len)?;
            Some(pointer)
        };
        Ok(Self { inline, heap, len })
    }

    /// Do not size the arena from an unverified native heap length. Typical
    /// names retain their single payload/terminator batch; larger names admit
    /// bounded readable chunks before growing the destination.
    pub fn copy_growing(
        &self,
        memory: &impl Memory,
        output: &mut Vec<u8>,
        start: usize,
        allocation: &'static str,
    ) -> Result<(), &'static str> {
        let end = start.checked_add(self.len).ok_or(allocation)?;
        if self.len <= 256 {
            let mut bytes = [0; 256];
            self.copy(memory, &mut bytes[..self.len])?;
            if end > output.len() {
                crate::allocation::reserve(output, end - output.len()).map_err(|_| allocation)?;
                output.resize(end, 0);
            }
            output[start..end].copy_from_slice(&bytes[..self.len]);
            return Ok(());
        }
        let pointer = self.heap.expect("large equipment string is heap allocated");
        msvc_string::copy_growing(
            memory,
            pointer,
            self.len,
            output,
            start,
            "equipment string bytes or terminator unreadable",
            allocation,
        )?;
        match read::<u8>(memory, at(pointer, self.len)?) {
            Some(0) => Ok(()),
            Some(_) => Err("equipment string terminator unreadable or invalid"),
            None => Err("equipment string bytes or terminator unreadable"),
        }
    }

    pub fn copy(&self, memory: &impl Memory, output: &mut [u8]) -> Result<(), &'static str> {
        if output.len() != self.len {
            return Err("equipment string destination length mismatch");
        }
        if let Some(pointer) = self.heap {
            let mut terminator = [0; 1];
            if memory.copy_scattered(
                [pointer, at(pointer, self.len)?],
                &mut [output, &mut terminator],
            ) != 2
            {
                return Err("equipment string bytes or terminator unreadable");
            }
            if terminator[0] != 0 {
                return Err("equipment string terminator unreadable or invalid");
            }
        } else {
            output.copy_from_slice(&self.inline[..self.len]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
