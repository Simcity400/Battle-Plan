//! CBuilding layout; the damage accumulator is signed engine fixed-point.

use crate::{
    Memory, cbuilding_template,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
};

/// Building vtable.
pub const VTABLE_RVA: usize = 0x0297_3b88;
/// Lexer type id, u32.
pub const TYPE_ID: usize = 0x08;
/// Total level, i16.
pub const LEVEL: usize = 0x40;
/// Healthy level, i16, compared with signed level requests.
pub const HEALTHY_LEVEL: usize = 0x42;
/// Partial health, signed i64 in fixed5.
pub const PARTIAL_HEALTH: usize = 0x48;
/// Repair completion threshold; reaching it clears partial health and raises the healthy level.
pub const PARTIAL_HEALTH_PER_LEVEL: i64 = 10_000_000;
/// CBuildingStatus pointer supplying location flags and id.
pub const BUILDING_STATUS: usize = 0x1d8;
/// Template pointer selecting localized names.
pub const TEMPLATE: usize = 0x1e0;
/// Signed-short level setter following status and template pointers.
pub const SET_LEVEL_RVA: usize = 0x010c_71a0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Native province or state id, disambiguated by the table and status flags.
    pub location: u32,
    pub building_type: u32,
    /// Position in the owning CBuildingStatus vector.
    pub ordinal: u32,
    pub status_flags: u32,
    pub country: u32,
    pub level: i16,
    pub healthy_level: i16,
    pub partial_health: i64,
    /// Listed in the owning status's active vector (`cbuilding_status::ACTIVE_BUILDINGS`).
    pub active: bool,
}

/// One authenticated location's building vector and the destination span it
/// owns in the caller's shared output. Ordinals are native vector positions.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LocationBuildings {
    pub buildings: Array,
    pub status: usize,
    pub indices: Array,
    /// The status's active vector, an ordered subset of `buildings`.
    pub active: Array,
    pub seed: Row,
    pub start: usize,
    pub len: usize,
}

impl LocationBuildings {
    /// An unstaged slot. It owns no output span and reads no native vector.
    pub(crate) const EMPTY: Self = Self {
        buildings: Array { data: 0, count: 0 },
        status: 0,
        indices: Array { data: 0, count: 0 },
        active: Array { data: 0, count: 0 },
        seed: Row {
            location: 0,
            building_type: 0,
            ordinal: 0,
            status_flags: 0,
            country: 0,
            level: 0,
            healthy_level: 0,
            partial_health: 0,
            active: false,
        },
        start: 0,
        len: 0,
    };
}

const UNLISTED: &str = "active building vector is not an ordered subset of the building vector";

/// Pointers gathered per guarded round.
const BATCH: usize = 64;
const POINTER: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const POINTER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&POINTER, 8).expect("valid record layout");

/// The staged locations' active vectors in location and native order, read
/// one guarded batch at a time as the building walk reaches them.
struct ActiveBuildings<'a> {
    locations: &'a [LocationBuildings],
    location: usize,
    index: u32,
    owners: [usize; BATCH],
    pointers: [usize; BATCH],
    len: usize,
    next: usize,
}

impl<'a> ActiveBuildings<'a> {
    fn new(locations: &'a [LocationBuildings]) -> Self {
        Self {
            locations,
            location: 0,
            index: 0,
            owners: [0; BATCH],
            pointers: [0; BATCH],
            len: 0,
            next: 0,
        }
    }

    /// The next unmatched entry as its location's position and pointer.
    fn head(&mut self, memory: &impl Memory) -> Result<Option<(usize, usize)>, &'static str> {
        if self.next == self.len {
            self.fill(memory)?;
        }
        Ok((self.next < self.len).then(|| (self.owners[self.next], self.pointers[self.next])))
    }

    /// Whether the building at `pointer` of location `owner` is the next
    /// entry; an earlier location's unmatched entry is not in its vector.
    fn take(
        &mut self,
        memory: &impl Memory,
        owner: usize,
        pointer: usize,
    ) -> Result<bool, &'static str> {
        match self.head(memory)? {
            Some((listed, _)) if listed < owner => Err(UNLISTED),
            Some((listed, entry)) if listed == owner && entry == pointer => {
                self.next += 1;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn fill(&mut self, memory: &impl Memory) -> Result<(), &'static str> {
        let mut addresses = [0_usize; BATCH];
        let mut count = 0;
        while self.location < self.locations.len() && count < BATCH {
            let active = self.locations[self.location].active;
            if self.index >= active.count {
                self.location += 1;
                self.index = 0;
                continue;
            }
            addresses[count] = active.element(self.index, 8).unwrap_or(0);
            self.owners[count] = self.location;
            self.index += 1;
            count += 1;
        }
        (self.next, self.len) = (0, 0);
        if count == 0 {
            return Ok(());
        }
        let mut raw = [0_u8; BATCH * 8];
        if memory.copy_records(
            RecordSources::Pointers(&addresses[..count]),
            &POINTER_LAYOUT,
            &mut raw,
        ) < count
        {
            return Err("active building pointer unreadable");
        }
        for (pointer, bytes) in self
            .pointers
            .iter_mut()
            .zip(raw.chunks_exact(8))
            .take(count)
        {
            *pointer = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                .map_err(|_| "active building pointer unreadable")?;
        }
        self.len = count;
        Ok(())
    }
}

/// Each dependent stage gathers a bounded batch using one fixed field layout.
/// Earlier rows finish validation before a later stage's failure is reported.
/// No pointers or template values survive this call, including a second walk.
///
/// A batch spans as many locations as it needs to fill, because most locations
/// hold only a handful of buildings and a per-location batch would pay four
/// guarded crossings for them. Every check still runs in location order and
/// then in native ordinal order, against that location's own status backlink,
/// index vector and already-written rows, so each reason and its position are
/// the ones a per-location read produced.
/// Destination storage grows only after acquiring a building's fixed fields;
/// a native vector count alone never causes a count-sized allocation.
///
/// The active vector is matched by pointer, in order, against the buildings
/// as they are read, so it must be an ordered subset of the building vector.
pub(crate) fn read_rows(
    memory: &impl Memory,
    base: usize,
    locations: &[LocationBuildings],
    output: &mut Vec<Row>,
) -> Result<(), &'static str> {
    const FIELDS: [RecordField; 5] = [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        },
        RecordField {
            source_offset: TYPE_ID,
            destination_offset: 8,
            len: 4,
        },
        RecordField {
            source_offset: LEVEL,
            destination_offset: 12,
            len: 4,
        },
        RecordField {
            source_offset: PARTIAL_HEALTH,
            destination_offset: 16,
            len: 8,
        },
        RecordField {
            source_offset: BUILDING_STATUS,
            destination_offset: 24,
            len: 16,
        },
    ];
    const TEMPLATE_FIELDS: [RecordField; 2] = [
        RecordField {
            source_offset: cbuilding_template::TYPE_ID,
            destination_offset: 0,
            len: 4,
        },
        RecordField {
            source_offset: cbuilding_template::DATABASE_ORDINAL,
            destination_offset: 4,
            len: 4,
        },
    ];
    const INDEX: [RecordField; 1] = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 4,
    }];
    const LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&FIELDS, 40).expect("valid record layout");
    const TEMPLATE_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&TEMPLATE_FIELDS, 8).expect("valid record layout");
    const INDEX_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&INDEX, 4).expect("valid record layout");
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    for location in locations {
        if location.len > location.buildings.count as usize {
            return Err("building output exceeds native vector");
        }
        if location.active.count > location.buildings.count {
            return Err("active building vector exceeds building vector");
        }
    }
    let mut active = ActiveBuildings::new(locations);
    let mut listed = [false; BATCH];
    let mut addresses = [0_usize; BATCH];
    let mut owners = [0_usize; BATCH];
    let mut ordinals = [0_u32; BATCH];
    let mut raw = [0_u8; BATCH * 40];
    let mut indirect = [0_u8; BATCH * 8];
    let mut location = 0;
    let mut ordinal = 0;
    loop {
        let mut count = 0;
        while location < locations.len() && count < BATCH {
            if ordinal >= locations[location].len {
                location += 1;
                ordinal = 0;
                continue;
            }
            addresses[count] = locations[location]
                .buildings
                .element(ordinal as u32, 8)
                .unwrap_or(0);
            owners[count] = location;
            ordinals[count] = ordinal as u32;
            ordinal += 1;
            count += 1;
        }
        if count == 0 {
            return match active.head(memory)? {
                Some(_) => Err(UNLISTED),
                None => Ok(()),
            };
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&addresses[..count]),
            &POINTER_LAYOUT,
            &mut indirect,
        );
        let mut valid = copied.min(count);
        let mut failure =
            (copied < count).then_some("building instance pointer unreadable or null");
        for (slot, address) in addresses[..valid].iter_mut().enumerate() {
            let pointer = u64::from_le_bytes(indirect[slot * 8..slot * 8 + 8].try_into().unwrap());
            if let Ok(pointer) = usize::try_from(pointer)
                && pointer != 0
            {
                *address = pointer;
            } else {
                valid = slot;
                failure = Some("building instance pointer unreadable or null");
                break;
            }
            match active.take(memory, owners[slot], *address) {
                Ok(matched) => listed[slot] = matched,
                Err(reason) => {
                    valid = slot;
                    failure = Some(reason);
                    break;
                }
            }
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&addresses[..valid]),
            &LAYOUT,
            &mut raw,
        );
        for (slot, fields) in raw.chunks_exact(40).take(valid).enumerate() {
            let completed = copied.saturating_sub(slot * FIELDS.len()).min(FIELDS.len());
            let owner = &locations[owners[slot]];
            let parsed = (|| {
                if completed == 0 {
                    return Err("building vtable unreadable");
                }
                if u64::from_le_bytes(fields[..8].try_into().unwrap()) != expected {
                    return Err("building vtable mismatch");
                }
                if completed != FIELDS.len() {
                    return Err("building fields unreadable");
                }
                if u64::from_le_bytes(fields[24..32].try_into().unwrap()) != owner.status as u64 {
                    return Err("building status backlink mismatch");
                }
                let template =
                    usize::try_from(u64::from_le_bytes(fields[32..40].try_into().unwrap()))
                        .ok()
                        .filter(|&pointer| pointer != 0)
                        .ok_or("building template pointer invalid")?;
                addresses[slot] = template;
                let written = owner.start + ordinals[slot] as usize;
                if written >= output.len() {
                    let required = written
                        .checked_add(1)
                        .ok_or("building row count overflow")?;
                    crate::allocation::reserve(output, required - output.len())
                        .map_err(|_| "building row allocation failed")?;
                    output.resize(required, Row::default());
                }
                output[written] = Row {
                    building_type: u32::from_le_bytes(fields[8..12].try_into().unwrap()),
                    ordinal: ordinals[slot],
                    level: i16::from_le_bytes(fields[12..14].try_into().unwrap()),
                    healthy_level: i16::from_le_bytes(fields[14..16].try_into().unwrap()),
                    partial_health: i64::from_le_bytes(fields[16..24].try_into().unwrap()),
                    active: listed[slot],
                    ..owner.seed
                };
                Ok(())
            })();
            if let Err(reason) = parsed {
                valid = slot;
                failure = Some(reason);
                break;
            }
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&addresses[..valid]),
            &TEMPLATE_LAYOUT,
            &mut indirect,
        );
        for (slot, fields) in indirect.chunks_exact(8).take(valid).enumerate() {
            let completed = copied
                .saturating_sub(slot * TEMPLATE_FIELDS.len())
                .min(TEMPLATE_FIELDS.len());
            let owner = &locations[owners[slot]];
            let parsed = (|| {
                if completed == 0
                    || u32::from_le_bytes(fields[..4].try_into().unwrap())
                        != output[owner.start + ordinals[slot] as usize].building_type
                {
                    return Err("building and template type mismatch");
                }
                if completed != TEMPLATE_FIELDS.len() {
                    return Err("building template ordinal unreadable");
                }
                let ordinal = u32::from_le_bytes(fields[4..8].try_into().unwrap());
                addresses[slot] = owner
                    .indices
                    .element(ordinal, 4)
                    .ok_or("building index map entry unreadable or outside vector")?;
                Ok(())
            })();
            if let Err(reason) = parsed {
                valid = slot;
                failure = Some(reason);
                break;
            }
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&addresses[..valid]),
            &INDEX_LAYOUT,
            &mut indirect,
        );
        for slot in 0..valid {
            if slot >= copied {
                return Err("building index map entry unreadable or outside vector");
            }
            let owner = &locations[owners[slot]];
            let index = i32::from_le_bytes(indirect[slot * 4..slot * 4 + 4].try_into().unwrap());
            if index
                != i32::try_from(ordinals[slot])
                    .map_err(|_| "building ordinal exceeds native range")?
            {
                return Err("building index map differs from pointer vector");
            }
            let written = owner.start + ordinals[slot] as usize;
            if output[owner.start..written]
                .iter()
                .any(|row| row.building_type == output[written].building_type)
            {
                return Err("duplicate building type in location");
            }
        }
        if let Some(reason) = failure {
            return Err(reason);
        }
    }
}
