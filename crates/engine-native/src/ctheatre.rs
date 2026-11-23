
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{
        Gathered,
        RecordField,
        RecordLayout,
        RecordSources,
        gather_records,
        packed_fields,
        packed_map,
        packed_stride,
    },
    msvc_string,
    read,
    read_slice,
    walk::{Buffer, Outcome},
};
use engine_native_parts::{
    cair_group,
    cair_theatre,
    carmy_group,
    ccontroller_area,
    ctheater_group,
};
use crate::{carmy, ccountry, corders_group, cprovince, ctask_force, cunit};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029f_22b8);
pub const REFERENCE_TYPE: u32 = 67;
pub const HANDLE: usize = 0x08;
pub const CONTROLLER_AREA_DATA: usize = 0x18;
pub const STATIONED_UNIT_DATA: usize = 0x68;
pub const ORDERS_GROUP_DATA: usize = 0x80;
pub const ARMY_GROUP_DATA: usize = 0x98;
pub const THEATER_GROUP_DATA: usize = 0xc8;
pub const OWNER: usize = 0x48;
pub const VOLUNTEERS_THEATRE: usize = 0x110;
pub const SIZE: usize = 0x1d0;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00f6_8bf0);

pub const HEADER_SPANS: [(usize, usize); 3] = [(0, 16), (OWNER, 4), (VOLUNTEERS_THEATRE, 4)];
const HEADER_FIELDS: [RecordField; 3] = packed_fields(HEADER_SPANS);
const HEADER_STRIDE: usize = packed_stride(HEADER_SPANS);
const HEADER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&HEADER_FIELDS, HEADER_STRIDE).expect("valid theatre header layout");
const HEADER_MAP: [u16; VOLUNTEERS_THEATRE + 4] = packed_map(HEADER_SPANS);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub owner: u32,
    pub volunteers_theatre: i32,
    pub ordinal: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ReservedLandWalk {
    pub len: Result<usize, &'static str>,
    pub native_count: Option<u32>,
    pub required_rows: usize,
    pub capacity_short: bool,
}
#[cfg(test)]
pub(crate) fn walk_reserved_land_headers(
    memory: &impl Memory,
    base: usize,
    root: Result<engine_native_memory::country_identity::Country, &'static str>,
    output: &mut Vec<Row>,
    validate_rosters: impl FnMut(usize, u64) -> Result<(), &'static str>,
) -> ReservedLandWalk {
    walk_reserved_land_headers_buffer(memory, base, root, Buffer::Growing(output), validate_rosters)
}


pub fn walk_reserved_land_headers_buffer(
    memory: &impl Memory,
    base: usize,
    root: Result<engine_native_memory::country_identity::Country, &'static str>,
    mut output: Buffer<'_, Row>,
    mut validate_rosters: impl FnMut(usize, u64) -> Result<(), &'static str>,
) -> ReservedLandWalk {
    let mut native_count = None;
    let mut required_rows = 0;
    let mut capacity_short = false;
    let mut written = 0;
    let len = (|| {
        let country = root?;
        if country.pointer == 0 || country.id != 0 || country.canonical_slot != 0 {
            return Err("reserved land root is not authenticated Country0");
        }
        let reader = Reader { memory, base };
        let theatres = reader.array(country.pointer, ccountry::THEATRES)?;
        native_count = Some(theatres.count);
        required_rows = theatres.count as usize;
        output.ensure(required_rows)?;
        if output.len() < required_rows {
            capacity_short = true;
            return Err("reserved land theatre row capacity insufficient");
        }
        for ordinal in 0..theatres.count {
            let theatre = reader.pointer(theatres, ordinal)?;
            let owner_address = theatre
                .checked_add(OWNER)
                .ok_or("reserved land theatre owner address overflow")?;
            let volunteers_address = theatre
                .checked_add(VOLUNTEERS_THEATRE)
                .ok_or("reserved land theatre volunteers address overflow")?;
            let mut identity = [0; 16];
            let mut owner = [0; 4];
            let mut volunteers = [0; 4];
            if memory.copy_scattered(
                [theatre, owner_address, volunteers_address],
                &mut [&mut identity, &mut owner, &mut volunteers],
            ) != HEADER_SPANS.len()
            {
                return Err("reserved land theatre header unreadable");
            }
            let id = reader.identity_from(theatre, Some(identity), VTABLE_RVA, REFERENCE_TYPE)?;
            if id >> 32 == 0 {
                return Err("reserved land theatre reference id is zero");
            }
            if u32::from_le_bytes(owner) != 0 {
                return Err("reserved land theatre owner is not zero");
            }
            validate_rosters(theatre, id)?;
            output[ordinal as usize] = Row {
                id,
                owner: 0,
                volunteers_theatre: i32::from_le_bytes(volunteers),
                ordinal,
            };
            written += 1;
        }
        Ok(required_rows)
    })();
    if len.is_err() {
        output[..written].fill(Row::default());
    }
    ReservedLandWalk {
        len,
        native_count,
        required_rows,
        capacity_short,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OrdersMembership {
    pub orders_group: u64,
    pub theater_group: u64,
}


const NOT_ROSTER_THEATRE: &str = "unit theatre is not one of its roster country's theatres";

pub fn unit_theatre(
    memory: &impl Memory,
    base: usize,
    country: usize,
    theatre: usize,
) -> Result<u64, &'static str> {
    if theatre == 0 {
        return Ok(0);
    }
    let reader = Reader { memory, base };
    let theatres = reader.array(country, ccountry::THEATRES)?;
    let mut found = None;
    for ordinal in 0..theatres.count {
        let pointer = reader.pointer(theatres, ordinal)?;
        let handle = reader.identity(pointer, VTABLE_RVA, REFERENCE_TYPE)?;
        if pointer == theatre {
            found = Some(handle);
        }
    }
    found.ok_or(NOT_ROSTER_THEATRE)
}

#[derive(Debug, Default)]
pub struct UnitTheatres {
    country: usize,
    read: Option<Result<(), &'static str>>,
    theatres: Vec<(usize, u64)>,
}

impl UnitTheatres {
    pub fn clear(&mut self) {
        self.country = 0;
        self.read = None;
        self.theatres.clear();
    }

    pub fn allocated_bytes(&self) -> u64 {
        (self.theatres.capacity() * size_of::<(usize, u64)>()) as u64
    }

    pub fn handle(
        &mut self,
        memory: &impl Memory,
        base: usize,
        country: usize,
        theatre: usize,
    ) -> Result<u64, &'static str> {
        if theatre == 0 {
            return Ok(0);
        }
        if self.read.is_none() || self.country != country {
            self.country = country;
            self.read = Some(self.read_country(memory, base, country));
        }
        self.read.expect("country theatres just read")?;
        self.theatres
            .iter()
            .find(|&&(pointer, _)| pointer == theatre)
            .map(|&(_, handle)| handle)
            .ok_or(NOT_ROSTER_THEATRE)
    }

    fn read_country(
        &mut self,
        memory: &impl Memory,
        base: usize,
        country: usize,
    ) -> Result<(), &'static str> {
        self.theatres.clear();
        let reader = Reader { memory, base };
        let theatres = reader.array(country, ccountry::THEATRES)?;
        engine_allocation::reserve(&mut self.theatres, theatres.count as usize)
            .map_err(|_| "unit theatre allocation failed")?;
        for ordinal in 0..theatres.count {
            let pointer = reader.pointer(theatres, ordinal)?;
            let handle = reader.identity(pointer, VTABLE_RVA, REFERENCE_TYPE)?;
            self.theatres.push((pointer, handle));
        }
        Ok(())
    }
}

pub fn read_orders_membership(
    memory: &impl Memory,
    base: usize,
    root: &corders_group::Root,
) -> Result<OrdersMembership, &'static str> {
    let reader = Reader { memory, base };
    let parent = reader.field::<u64>(root.pointer, corders_group::THEATER_GROUP)?;
    let theater_group = if parent == 0 {
        0
    } else {
        reader.identity(
            usize::try_from(parent).map_err(|_| "orders theatre group pointer overflow")?,
            ctheater_group::VTABLE_RVA,
            ctheater_group::REFERENCE_TYPE,
        )?
    };
    Ok(OrdersMembership {
        orders_group: root.handle,
        theater_group,
    })
}

pub struct Buffers<'a> {
    pub land: &'a mut Vec<Row>,
    pub land_groups: &'a mut Vec<ctheater_group::Row>,
    pub land_members: &'a mut Vec<ctheater_group::MemberRow>,
    pub air: &'a mut Vec<cair_theatre::Row>,
    pub air_groups: &'a mut Vec<cair_group::Row>,
    pub air_members: &'a mut Vec<cair_group::MemberRow>,
    pub names: &'a mut Vec<u8>,
}

pub struct Storage<'a> {
    pub land: Buffer<'a, Row>,
    pub land_groups: Buffer<'a, ctheater_group::Row>,
    pub land_members: Buffer<'a, ctheater_group::MemberRow>,
    pub air: Buffer<'a, cair_theatre::Row>,
    pub air_groups: Buffer<'a, cair_group::Row>,
    pub air_members: Buffer<'a, cair_group::MemberRow>,
    pub names: Buffer<'a, u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub land: Outcome,
    pub land_groups: Outcome,
    pub land_members: Outcome,
    pub air: Outcome,
    pub air_groups: Outcome,
    pub air_members: Outcome,
    pub names_len: usize,
    pub names_required: usize,
}

#[derive(Default, Clone, Copy)]
struct Count {
    len: usize,
    unread: u32,
}

impl Count {
    fn omit(&mut self) -> Result<(), &'static str> {
        self.unread = self
            .unread
            .checked_add(1)
            .ok_or("theatre row count overflow")?;
        Ok(())
    }

    fn push<T>(&mut self, output: &mut Buffer<'_, T>, row: T) -> Result<(), &'static str> {
        if let Some(destination) = output.get_mut(self.len) {
            *destination = row;
            self.len += 1;
            Ok(())
        } else {
            self.omit()
        }
    }

    fn outcome(self, failure: Option<&'static str>) -> Outcome {
        if let Some(reason) = failure {
            Outcome::Unavailable { reason }
        } else if self.unread == 0 {
            Outcome::Rows { len: self.len }
        } else {
            Outcome::Truncated {
                len: self.len,
                unread_rows: self.unread,
            }
        }
    }
}

struct Names<'a> {
    bytes: Buffer<'a, u8>,
    len: usize,
    required: usize,
}

impl Names<'_> {
    fn copy(
        &mut self,
        memory: &impl Memory,
        address: usize,
        row_fits: bool,
    ) -> Result<Option<(usize, usize)>, &'static str> {
        let header = read::<[u8; msvc_string::STRIDE]>(memory, address)
            .ok_or("theatre name header unreadable")?;
        let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
        let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
        if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
            return Err("theatre name header invalid");
        }
        let len = usize::try_from(len).map_err(|_| "theatre name length overflow")?;
        let source = if capacity == msvc_string::INLINE_CAPACITY {
            address
        } else {
            usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
                .ok()
                .filter(|&pointer| pointer != 0)
                .ok_or("theatre name pointer null or overflow")?
        };
        let terminator = source
            .checked_add(len)
            .ok_or("theatre name address overflow")?;
        if read::<u8>(memory, terminator) != Some(0) {
            return Err("theatre name terminator unreadable or invalid");
        }
        self.required = self
            .required
            .checked_add(len)
            .ok_or("theatre name demand overflow")?;
        self.bytes.ensure(self.required)?;
        if !row_fits || len > self.bytes.len() - self.len {
            return Ok(None);
        }
        let start = self.len;
        if !read_slice(memory, source, &mut self.bytes[start..start + len]) {
            return Err("theatre name bytes unreadable");
        }
        self.len += len;
        Ok(Some((start, len)))
    }
}

struct LandDetails {
    active: bool,
    failure: Option<&'static str>,
}

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
}

impl<M: Memory> Reader<'_, M> {
    fn field<T: engine_native_memory::memory::Raw>(
        &self,
        pointer: usize,
        offset: usize,
    ) -> Result<T, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| read(self.memory, address))
            .ok_or("theatre field unreadable or address overflow")
    }

    fn array(&self, pointer: usize, offset: usize) -> Result<Array, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| Array::read(self.memory, address, 8))
            .ok_or("theatre vector unreadable or invalid")
    }

    fn pointer(&self, array: Array, index: u32) -> Result<usize, &'static str> {
        array
            .element(index, 8)
            .and_then(|address| read::<u64>(self.memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("theatre vector pointer unreadable or null")
    }

    fn identity(
        &self,
        pointer: usize,
        vtable: Rva,
        reference_type: u32,
    ) -> Result<u64, &'static str> {
        self.identity_from(pointer, None, vtable, reference_type)
    }

    fn identity_from(
        &self,
        pointer: usize,
        gathered: Option<[u8; 16]>,
        vtable: Rva,
        reference_type: u32,
    ) -> Result<u64, &'static str> {
        let header = match gathered {
            Some(header) => header,
            None => read::<[u8; 16]>(self.memory, pointer).ok_or("theatre identity unreadable")?,
        };
        let expected = engine_sites::at(self.base, vtable).ok_or("module address overflow")? as u64;
        if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected {
            return Err("theatre object vtable mismatch");
        }
        let id = u64::from_le_bytes(header[8..].try_into().unwrap());
        if id as u32 != reference_type {
            return Err("theatre object reference type mismatch");
        }
        Ok(id)
    }

    fn land(
        &self,
        country: engine_native_memory::country_identity::Country,
        countries: &ccountry::Reader<'_, M>,
        details: &mut LandDetails,
        buffers: &mut Storage<'_>,
        names: &mut Names<'_>,
        counts: &mut [Count; 6],
    ) -> Result<(), &'static str> {
        let theatres = self.array(country.pointer, ccountry::THEATRES)?;
        buffers.land.ensure(
            counts[0]
                .len
                .checked_add(theatres.count as usize)
                .ok_or("land theatre demand overflow")?,
        )?;
        let mut record = [0_u8; HEADER_STRIDE];
        let mut complete = [false];
        for ordinal in 0..theatres.count {
            let theatre = self.pointer(theatres, ordinal)?;
            gather_records(
                self.memory,
                &[theatre],
                &HEADER_LAYOUT,
                &mut record,
                &mut complete,
            );
            let gathered = Gathered::row(&HEADER_LAYOUT, &HEADER_MAP, &record, &complete, 0);
            let id = self.identity_from(theatre, gathered.get(0), VTABLE_RVA, REFERENCE_TYPE)?;
            let owner = match gathered.get::<u32>(OWNER) {
                Some(owner) => owner,
                None => self.field::<u32>(theatre, OWNER)?,
            };
            let volunteers_theatre = match gathered.get::<i32>(VOLUNTEERS_THEATRE) {
                Some(volunteers) => volunteers,
                None => self.field::<i32>(theatre, VOLUNTEERS_THEATRE)?,
            };
            if countries.resolve(owner)?.pointer != country.pointer {
                return Err("land theatre owner differs from country roster");
            }
            counts[0].push(
                &mut buffers.land,
                Row {
                    id,
                    owner,
                    volunteers_theatre,
                    ordinal,
                },
            )?;
            if !details.active || details.failure.is_some() {
                continue;
            }
            let detail = (|| {
                let groups = self.array(theatre, THEATER_GROUP_DATA)?;
                buffers.land_groups.ensure(
                    counts[1]
                        .len
                        .checked_add(groups.count as usize)
                        .ok_or("land theatre group demand overflow")?,
                )?;
                for group_ordinal in 0..groups.count {
                    let group = self.pointer(groups, group_ordinal)?;
                    let group_id = self.identity(
                        group,
                        ctheater_group::VTABLE_RVA,
                        ctheater_group::REFERENCE_TYPE,
                    )?;
                    if self.field::<u64>(group, ctheater_group::THEATRE)? != theatre as u64 {
                        return Err("land group parent differs from theatre roster");
                    }
                    let name_address = group
                        .checked_add(ctheater_group::NAME)
                        .ok_or("theatre name address overflow")?;
                    if let Some((name_start, name_len)) = names.copy(
                        self.memory,
                        name_address,
                        counts[1].len < buffers.land_groups.len(),
                    )? {
                        let priority = self.field::<i32>(group, ctheater_group::PRIORITY)?;
                        counts[1].push(
                            &mut buffers.land_groups,
                            ctheater_group::Row {
                                id: group_id,
                                theatre: id,
                                ordinal: group_ordinal,
                                priority,
                                name_start,
                                name_len,
                            },
                        )?;
                    } else {
                        counts[1].omit()?;
                    }
                    let members = self.array(group, ctheater_group::ORDERS_GROUP_DATA)?;
                    buffers.land_members.ensure(
                        counts[2]
                            .len
                            .checked_add(members.count as usize)
                            .ok_or("land theatre member demand overflow")?,
                    )?;
                    for member_ordinal in 0..members.count {
                        let member = self.pointer(members, member_ordinal)?;
                        let vtable = self.field::<u64>(member, 0)?;
                        let normal = engine_sites::at(self.base, corders_group::VTABLE_RVA)
                            .ok_or("module address overflow")?
                            as u64;
                        let army_group = engine_sites::at(self.base, carmy_group::VTABLE_RVA)
                            .ok_or("module address overflow")?
                            as u64;
                        let expected_vtable = if vtable == normal {
                            corders_group::VTABLE_RVA
                        } else if vtable == army_group {
                            carmy_group::VTABLE_RVA
                        } else {
                            return Err("theatre member is not an orders group");
                        };
                        let member_id =
                            self.identity(member, expected_vtable, corders_group::REFERENCE_TYPE)?;
                        if self.field::<u64>(member, corders_group::THEATER_GROUP)? != group as u64
                            || self.field::<u64>(member, corders_group::THEATRE)? != theatre as u64
                        {
                            return Err("orders group parents differ from theatre membership");
                        }
                        counts[2].push(
                            &mut buffers.land_members,
                            ctheater_group::MemberRow {
                                orders_group: member_id,
                                theater_group: group_id,
                                ordinal: member_ordinal,
                            },
                        )?;
                    }
                }
                Ok::<(), &'static str>(())
            })();
            if let Err(reason) = detail {
                details.failure = Some(reason);
            }
        }
        Ok(())
    }

    fn air(
        &self,
        country: engine_native_memory::country_identity::Country,
        countries: &ccountry::Reader<'_, M>,
        buffers: &mut Storage<'_>,
        names: &mut Names<'_>,
        counts: &mut [Count; 6],
    ) -> Result<(), &'static str> {
        let theatres = self.array(country.pointer, ccountry::AIR_THEATRES)?;
        buffers.air.ensure(
            counts[3]
                .len
                .checked_add(theatres.count as usize)
                .ok_or("air theatre demand overflow")?,
        )?;
        for ordinal in 0..theatres.count {
            let theatre = self.pointer(theatres, ordinal)?;
            let id = self.identity(theatre, cair_theatre::VTABLE_RVA, cair_theatre::TYPE)?;
            let owner = self.field::<u32>(theatre, cair_theatre::COUNTRY)?;
            if countries.resolve(owner)?.pointer != country.pointer {
                return Err("air theatre owner differs from country roster");
            }
            let name_address = theatre
                .checked_add(cair_theatre::NAME)
                .ok_or("theatre name address overflow")?;
            if let Some((name_start, name_len)) =
                names.copy(self.memory, name_address, counts[3].len < buffers.air.len())?
            {
                counts[3].push(
                    &mut buffers.air,
                    cair_theatre::Row {
                        id,
                        owner,
                        ordinal,
                        name_start,
                        name_len,
                    },
                )?;
            } else {
                counts[3].omit()?;
            }
            let groups = self.array(theatre, cair_theatre::GROUPS)?;
            buffers.air_groups.ensure(
                counts[4]
                    .len
                    .checked_add(groups.count as usize)
                    .ok_or("air theatre group demand overflow")?,
            )?;
            for group_ordinal in 0..groups.count {
                let group = self.pointer(groups, group_ordinal)?;
                let group_id = self.identity(group, cair_group::VTABLE_RVA, cair_group::TYPE)?;
                if self.field::<u64>(group, cair_group::THEATRE_HANDLE)? != id {
                    return Err("air group parent differs from theatre roster");
                }
                let name_address = group
                    .checked_add(cair_group::NAME)
                    .ok_or("theatre name address overflow")?;
                if let Some((name_start, name_len)) = names.copy(
                    self.memory,
                    name_address,
                    counts[4].len < buffers.air_groups.len(),
                )? {
                    counts[4].push(
                        &mut buffers.air_groups,
                        cair_group::Row {
                            id: group_id,
                            theatre: id,
                            ordinal: group_ordinal,
                            name_start,
                            name_len,
                        },
                    )?;
                } else {
                    counts[4].omit()?;
                }
                let wings = self.array(group, cair_group::WINGS)?;
                buffers.air_members.ensure(
                    counts[5]
                        .len
                        .checked_add(wings.count as usize)
                        .ok_or("air theatre member demand overflow")?,
                )?;
                for wing_ordinal in 0..wings.count {
                    let address = wings
                        .element(wing_ordinal, 8)
                        .ok_or("air wing reference address overflow")?;
                    let wing =
                        read::<u64>(self.memory, address).ok_or("air wing reference unreadable")?;
                    if wing as u32 != engine_native_parts::cair_wing_layout::TYPE {
                        return Err("air group member reference is not an air wing");
                    }
                    counts[5].push(
                        &mut buffers.air_members,
                        cair_group::MemberRow {
                            wing,
                            group: group_id,
                            ordinal: wing_ordinal,
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    details_active: bool,
    buffers: Buffers<'_>,
) -> Walk {
    walk_buffers(
        memory,
        base,
        game_state,
        details_active,
        Storage {
            land: Buffer::Growing(buffers.land),
            land_groups: Buffer::Growing(buffers.land_groups),
            land_members: Buffer::Growing(buffers.land_members),
            air: Buffer::Growing(buffers.air),
            air_groups: Buffer::Growing(buffers.air_groups),
            air_members: Buffer::Growing(buffers.air_members),
            names: Buffer::Growing(buffers.names),
        },
    )
}

pub fn walk_buffers(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    details_active: bool,
    mut buffers: Storage<'_>,
) -> Walk {
    let mut counts = [Count::default(); 6];
    let mut land_failure = None;
    let mut land_details = LandDetails {
        active: details_active,
        failure: None,
    };
    let mut air_failure = None;
    let mut names = Names {
        bytes: std::mem::replace(&mut buffers.names, Buffer::Count),
        len: 0,
        required: 0,
    };
    let reader = Reader { memory, base };
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        for index in 1..countries.count() {
            let Some(country) = countries.at(index)? else {
                continue;
            };
            if land_failure.is_none() {
                land_failure = reader
                    .land(
                        country,
                        &countries,
                        &mut land_details,
                        &mut buffers,
                        &mut names,
                        &mut counts,
                    )
                    .err();
            }
            if details_active && air_failure.is_none() {
                air_failure = reader
                    .air(country, &countries, &mut buffers, &mut names, &mut counts)
                    .err();
            }
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = result {
        land_failure = Some(reason);
        air_failure = Some(reason);
    }
    Walk {
        land: counts[0].outcome(land_failure),
        land_groups: counts[1].outcome(land_details.failure.or(land_failure)),
        land_members: counts[2].outcome(land_details.failure.or(land_failure)),
        air: counts[3].outcome(air_failure),
        air_groups: counts[4].outcome(air_failure),
        air_members: counts[5].outcome(air_failure),
        names_len: names.len,
        names_required: names.required,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StationedUnitRow {
    pub theatre: u64,
    pub ordinal: u32,
    pub unit: u64,
}

pub struct StationedBuffers<'a> {
    pub areas_active: bool,
    pub areas: &'a mut Vec<ccontroller_area::Row>,
    pub area_provinces: &'a mut Vec<u32>,
    pub units: &'a mut Vec<StationedUnitRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StationedWalk {
    pub areas: Outcome,
    pub units: Outcome,
    pub provinces_len: usize,
    pub required_provinces: usize,
}

pub struct StationedStorage<'a> {
    pub areas_active: bool,
    pub areas: Buffer<'a, ccontroller_area::Row>,
    pub area_provinces: Buffer<'a, u32>,
    pub units: Buffer<'a, StationedUnitRow>,
}

pub fn walk_stationed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    buffers: StationedBuffers<'_>,
) -> StationedWalk {
    walk_stationed_buffers(
        memory,
        base,
        game_state,
        StationedStorage {
            areas_active: buffers.areas_active,
            areas: Buffer::Growing(buffers.areas),
            area_provinces: Buffer::Growing(buffers.area_provinces),
            units: Buffer::Growing(buffers.units),
        },
    )
}

pub fn count_stationed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    areas_active: bool,
) -> StationedWalk {
    walk_stationed_buffers(
        memory,
        base,
        game_state,
        StationedStorage {
            areas_active,
            areas: Buffer::Count,
            area_provinces: Buffer::Count,
            units: Buffer::Count,
        },
    )
}

pub fn walk_stationed_buffers(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut buffers: StationedStorage<'_>,
) -> StationedWalk {
    let mut counts = [Count::default(); 2];
    let mut area_failure = None;
    let mut unit_failure = None;
    let mut provinces_len = 0;
    let mut required_provinces = 0_usize;
    let reader = Reader { memory, base };
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let theatres = reader.array(country.pointer, ccountry::THEATRES)?;
            for ordinal in 0..theatres.count {
                let theatre = reader.pointer(theatres, ordinal)?;
                let id = reader.identity(theatre, VTABLE_RVA, REFERENCE_TYPE)?;
                if reader.field::<i32>(theatre, VOLUNTEERS_THEATRE)? <= 0 {
                    continue;
                }
                if buffers.areas_active && area_failure.is_none() {
                    area_failure = reader
                        .areas(
                            theatre,
                            id,
                            &mut buffers,
                            &mut counts[0],
                            &mut provinces_len,
                            &mut required_provinces,
                        )
                        .err();
                }
                if unit_failure.is_none() {
                    unit_failure = reader
                        .stationed_units(theatre, id, &mut buffers, &mut counts[1])
                        .err();
                }
            }
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = result {
        area_failure = Some(reason);
        unit_failure = Some(reason);
    }
    StationedWalk {
        areas: counts[0].outcome(area_failure),
        units: counts[1].outcome(unit_failure),
        provinces_len,
        required_provinces,
    }
}

impl<M: Memory> Reader<'_, M> {
    fn areas(
        &self,
        theatre: usize,
        id: u64,
        buffers: &mut StationedStorage<'_>,
        count: &mut Count,
        provinces_len: &mut usize,
        required_provinces: &mut usize,
    ) -> Result<(), &'static str> {
        const FIELDS: [RecordField; 2] = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 8,
            },
            RecordField {
                source_offset: cprovince::ID,
                destination_offset: 8,
                len: 4,
            },
        ];
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&FIELDS, 16).expect("valid record layout");
        let areas = self.array(theatre, CONTROLLER_AREA_DATA)?;
        buffers.areas.ensure(
            count
                .len
                .checked_add(areas.count as usize)
                .ok_or("controller area row demand overflow")?,
        )?;
        for ordinal in 0..areas.count {
            let area = self
                .pointer(areas, ordinal)
                .map_err(|_| "theatre controller area pointer unreadable or null")?;
            let expected = engine_sites::at(self.base, ccontroller_area::VTABLE_RVA)
                .ok_or("module address overflow")?;
            if read::<u64>(self.memory, area) != Some(expected as u64) {
                return Err("theatre controller area vtable mismatch");
            }
            let representative = self
                .field::<u64>(area, ccontroller_area::REPRESENTATIVE_PROVINCE)
                .and_then(|pointer| {
                    usize::try_from(pointer).map_err(|_| "controller area address overflow")
                })?;
            let representative = match representative {
                0 => None,
                pointer => Some(self.province_id(pointer)?),
            };
            let members = self
                .array(area, ccontroller_area::PROVINCE_DATA)
                .map_err(|_| "controller area province vector unreadable or invalid")?;
            *required_provinces = required_provinces
                .checked_add(members.count as usize)
                .ok_or("controller area province demand overflow")?;
            buffers.area_provinces.ensure(*required_provinces)?;
            if count.len == buffers.areas.len()
                || members.count as usize > buffers.area_provinces.len() - *provinces_len
            {
                count.omit()?;
                continue;
            }
            let start = *provinces_len;
            const BATCH: usize = 32;
            let mut pointers = [0_usize; BATCH];
            let mut raw = [0_u8; BATCH * 8];
            let mut identities = [0_u8; BATCH * 16];
            let mut index = 0;
            while index < members.count {
                let count = ((members.count - index) as usize).min(BATCH);
                self.pointer_batch(members, index, &mut pointers[..count], &mut raw)?;
                if self.memory.copy_records(
                    RecordSources::Pointers(&pointers[..count]),
                    &LAYOUT,
                    &mut identities[..count * 16],
                ) != count * FIELDS.len()
                {
                    return Err("controller area province fields unreadable");
                }
                let expected = engine_sites::at(self.base, cprovince::VTABLE_RVA)
                    .ok_or("module address overflow")?;
                for record in identities[..count * 16].chunks_exact(16) {
                    if u64::from_le_bytes(record[..8].try_into().expect("eight bytes"))
                        != expected as u64
                    {
                        return Err("theatre province vtable mismatch");
                    }
                    let id = u32::from_le_bytes(record[8..12].try_into().expect("four bytes"));
                    if id == 0 {
                        return Err("theatre province id is zero");
                    }
                    buffers.area_provinces[*provinces_len] = id;
                    *provinces_len += 1;
                }
                index += count as u32;
            }
            count.push(
                &mut buffers.areas,
                ccontroller_area::Row {
                    theatre: id,
                    ordinal,
                    representative,
                    provinces_start: start,
                    provinces_len: members.count as usize,
                },
            )?;
        }
        Ok(())
    }

    fn pointer_batch(
        &self,
        array: Array,
        first: u32,
        output: &mut [usize],
        bytes: &mut [u8],
    ) -> Result<(), &'static str> {
        let window = array
            .slice(first, output.len() as u32, 8)
            .map_err(|_| "theatre vector window outside its own count")?;
        let bytes = bytes
            .get_mut(..output.len() * 8)
            .ok_or("theatre vector window exceeds its batch buffer")?;
        if !engine_native_memory::read_slice(self.memory, window.data, bytes) {
            return Err("controller area province pointer unreadable or null");
        }
        for (destination, source) in output.iter_mut().zip(bytes.chunks_exact(8)) {
            *destination =
                usize::try_from(u64::from_le_bytes(source.try_into().expect("eight bytes")))
                    .ok()
                    .filter(|pointer| *pointer != 0)
                    .ok_or("controller area province pointer unreadable or null")?;
        }
        Ok(())
    }

    fn province_id(&self, province: usize) -> Result<u32, &'static str> {
        let expected =
            engine_sites::at(self.base, cprovince::VTABLE_RVA).ok_or("module address overflow")?;
        if read::<u64>(self.memory, province) != Some(expected as u64) {
            return Err("theatre province vtable mismatch");
        }
        let id = self.field::<u32>(province, cprovince::ID)?;
        if id == 0 {
            return Err("theatre province id is zero");
        }
        Ok(id)
    }

    fn stationed_units(
        &self,
        theatre: usize,
        id: u64,
        buffers: &mut StationedStorage<'_>,
        count: &mut Count,
    ) -> Result<(), &'static str> {
        let units = self.array(theatre, STATIONED_UNIT_DATA)?;
        buffers.units.ensure(
            count
                .len
                .checked_add(units.count as usize)
                .ok_or("stationed unit demand overflow")?,
        )?;
        for ordinal in 0..units.count {
            let unit = self
                .pointer(units, ordinal)
                .map_err(|_| "stationed unit pointer unreadable or null")?;
            let vtable = read::<u64>(self.memory, unit);
            if ![carmy::VTABLE_RVA, ctask_force::VTABLE_RVA]
                .into_iter()
                .any(|rva| {
                    engine_sites::at(self.base, rva)
                        .is_some_and(|expected| vtable == Some(expected as u64))
                })
            {
                return Err("stationed unit is neither a division nor a task force");
            }
            let handle = self.field::<u64>(unit, cunit::HANDLE)?;
            if handle == 0 {
                return Err("stationed unit has a null reference handle");
            }
            count.push(
                &mut buffers.units,
                StationedUnitRow {
                    theatre: id,
                    ordinal,
                    unit: handle,
                },
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
