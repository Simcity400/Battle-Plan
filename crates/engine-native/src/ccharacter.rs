pub mod template;
pub mod manager;
pub use engine_native_memory::msvc_string::{Literal};

use crate::{
    Memory,
    ccharacter::{manager as ccharacter_manager, template as ccharacter_template},
    read,
    read_slice,
    walk::Outcome,
};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029f_b0d0);
pub const REFERENCE_TYPE: u32 = 73;
pub const TYPE: usize = 0x08;
pub const ID: usize = 0x0c;
pub const NAME_TOKEN_ID: usize = 0x18;
pub const TEMPLATE: usize = 0x20;
pub const NAME: usize = 0x28;
pub const TOKEN: usize = 0x48;
pub const COUNTRY: usize = 0x68;
pub const GENDER: usize = 0x70;
pub const PORTRAITS: usize = 0x78;
pub const PORTRAIT_DATA: usize = 0x80;
#[cfg(any(test, feature = "test-support"))]
pub const PORTRAIT_COUNT: usize = 0x8c;
pub const LEADER_MAP_HEAD: usize = 0x98;
pub const LEADER_MAP_SIZE: usize = 0xa0;
pub const UNIT_LEADER: usize = 0xa8;
pub const ADVISOR_MAP_HEAD: usize = 0xb0;
pub const ADVISOR_MAP_SIZE: usize = 0xb8;
pub const SCIENTIST: usize = 0xc0;
pub const OPERATIVE: usize = 0xc8;
pub const VARIABLES: usize = 0xd8;
pub const FLAGS: usize = 0x110;
pub const VIEW_EXTENT: usize = ADVISOR_MAP_SIZE + 8;
pub const NAME_TOKEN_SPAN: usize = NAME - NAME_TOKEN_ID;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub country: u32,
    pub name_token_id: i32,
    pub gender: u32,
    pub roster: u8,
    pub ordinal: u32,
    pub name_start: usize,
    pub name_len: usize,
    pub token_start: usize,
    pub token_len: usize,
    pub template: usize,
    pub template_identity: ccharacter_template::Identity,
    pub template_file_start: usize,
    pub template_file_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Occurrence {
    pub pointer: usize,
    pub id: u32,
    pub roster: u8,
    pub ordinal: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AcquiredFields {
    pub country: Option<[u8; 4]>,
    pub leaders: Option<[u8; 16]>,
    pub unit: Option<[u8; 8]>,
    pub advisors: Option<[u8; 16]>,
}

pub struct View<'a, M: Memory> {
    memory: &'a M,
    pointer: usize,
    roster: u8,
    ordinal: u32,
    identity: [u8; 16],
    country: Option<[u8; 4]>,
    name_token_id: Option<[u8; NAME_TOKEN_SPAN]>,
    names: Option<[u8; 64]>,
    gender: Option<[u8; 4]>,
    leaders: Option<[u8; 16]>,
    unit: Option<[u8; 8]>,
    advisors: Option<[u8; 16]>,
}

impl<'a, M: Memory> View<'a, M> {
    pub(crate) fn read(
        memory: &'a M,
        base: usize,
        pointer: usize,
        roster: u8,
        ordinal: u32,
        required: [bool; 4],
    ) -> Result<Self, &'static str> {
        let mut identity = [0; 16];
        let mut country = [0; 4];
        let mut name_token_id = [0; NAME_TOKEN_SPAN];
        let mut names = [0; 64];
        let mut leaders = [0; 16];
        let mut unit = [0; 8];
        let mut advisors = [0; 16];
        let mut gender = [0; 4];
        let enabled = [
            true,
            required[0] || required[1] || required[3],
            required[0],
            required[0],
            required[1],
            required[2],
            required[3],
            required[0],
        ];
        let offsets = [
            0,
            COUNTRY,
            NAME_TOKEN_ID,
            NAME,
            LEADER_MAP_HEAD,
            UNIT_LEADER,
            ADVISOR_MAP_HEAD,
            GENDER,
        ];
        let addresses = offsets.map(|offset| pointer.checked_add(offset).unwrap_or(0));
        let mut spans: [&mut [u8]; 8] = [
            &mut identity,
            &mut country,
            &mut name_token_id,
            &mut names,
            &mut leaders,
            &mut unit,
            &mut advisors,
            &mut gender,
        ];
        for (span, enabled) in spans.iter_mut().zip(enabled) {
            if !enabled {
                *span = &mut [];
            }
        }
        let completed = memory.copy_scattered(addresses, &mut spans);
        if completed == 0 {
            return Err("character identity unreadable");
        }
        let mut copied =
            std::array::from_fn::<_, 8, _>(|index| enabled[index] && index < completed);
        for index in completed.saturating_add(1)..spans.len() {
            if enabled[index] {
                copied[index] = read_slice(memory, addresses[index], spans[index]);
            }
        }
        let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
        if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64
            || u32::from_le_bytes(identity[TYPE..TYPE + 4].try_into().unwrap()) != REFERENCE_TYPE
        {
            return Err("character vtable or reference type mismatch");
        }
        Ok(Self {
            memory,
            pointer,
            roster,
            ordinal,
            identity,
            country: copied[1].then_some(country),
            name_token_id: copied[2].then_some(name_token_id),
            names: copied[3].then_some(names),
            leaders: copied[4].then_some(leaders),
            unit: copied[5].then_some(unit),
            advisors: copied[6].then_some(advisors),
            gender: copied[7].then_some(gender),
        })
    }

    pub fn acquired(
        memory: &'a M,
        base: usize,
        occurrence: Occurrence,
        fields: AcquiredFields,
    ) -> Result<Self, &'static str> {
        let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
        let mut identity = [0; 16];
        identity[..8].copy_from_slice(&(expected as u64).to_le_bytes());
        identity[TYPE..TYPE + 4].copy_from_slice(&REFERENCE_TYPE.to_le_bytes());
        identity[ID..ID + 4].copy_from_slice(&occurrence.id.to_le_bytes());
        Ok(Self {
            memory,
            pointer: occurrence.pointer,
            roster: occurrence.roster,
            ordinal: occurrence.ordinal,
            identity,
            country: fields.country,
            name_token_id: None,
            names: None,
            gender: None,
            leaders: fields.leaders,
            unit: fields.unit,
            advisors: fields.advisors,
        })
    }

    pub fn reference(&self) -> (usize, u32) {
        (
            self.pointer,
            u32::from_le_bytes(self.identity[ID..ID + 4].try_into().unwrap()),
        )
    }

    pub fn entry(&self) -> Result<ccharacter_manager::Entry, &'static str> {
        let (pointer, id) = self.reference();
        Ok(ccharacter_manager::Entry {
            pointer,
            id,
            country: u32::from_le_bytes(self.country.ok_or("character country unreadable")?),
            roster: self.roster,
            ordinal: self.ordinal,
        })
    }

    fn copied(&self, address: usize, len: usize) -> Result<Option<&[u8]>, ()> {
        let end = address.checked_add(len).ok_or(())?;
        let object_end = self.pointer.saturating_add(VIEW_EXTENT);
        if end <= self.pointer || address >= object_end {
            return Ok(None);
        }
        let start = address.checked_sub(self.pointer).ok_or(())?;
        let end = start.checked_add(len).ok_or(())?;
        macro_rules! field {
            ($offset:expr, $size:expr, $bytes:expr) => {
                if let Some(from) = start.checked_sub($offset)
                    && end <= $offset + $size
                {
                    return $bytes
                        .as_ref()
                        .and_then(|bytes| bytes.get(from..end - $offset))
                        .map(Some)
                        .ok_or(());
                }
            };
        }
        field!(0, 16, Some(&self.identity));
        field!(COUNTRY, 4, self.country);
        field!(NAME_TOKEN_ID, NAME_TOKEN_SPAN, self.name_token_id);
        field!(NAME, 64, self.names);
        field!(LEADER_MAP_HEAD, 16, self.leaders);
        field!(UNIT_LEADER, 8, self.unit);
        field!(ADVISOR_MAP_HEAD, 16, self.advisors);
        field!(GENDER, 4, self.gender);
        Err(())
    }
}

impl<M: Memory> Memory for View<'_, M> {
    #[inline]
    fn read_scalar<T: crate::memory::Raw>(&self, address: usize) -> Option<T> {
        if size_of::<T>() == 0 {
            return Some(T::ZERO);
        }
        match self.copied(address, size_of::<T>()) {
            Ok(Some(source)) => crate::memory::scalar_from_bytes(source),
            Ok(None) => self.memory.read_scalar(address),
            Err(()) => None,
        }
    }

    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return true;
        }
        match self.copied(address, destination.len()) {
            Ok(Some(source)) => {
                destination.copy_from_slice(source);
                true
            }
            Ok(None) => self.memory.copy(address, destination),
            Err(()) => false,
        }
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        if addresses
            .iter()
            .zip(destinations.iter())
            .all(|(&address, destination)| {
                matches!(self.copied(address, destination.len()), Ok(None))
            })
        {
            return self.memory.copy_scattered(addresses, destinations);
        }
        for (index, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if !destination.is_empty()
                && (address == 0
                    || address.checked_add(destination.len()).is_none()
                    || !self.copy(address, destination))
            {
                return index;
            }
        }
        N
    }
}

pub fn walk_entry<'k>(
    memory: &impl Memory,
    base: usize,
    entry: ccharacter_manager::Entry,
    known: impl FnOnce(usize) -> Option<ccharacter_template::Known<'k>>,
    mut output: crate::walk::Buffer<'_, Row>,
    mut bytes: crate::walk::Buffer<'_, u8>,
) -> NameWalk {
    let result = (|| {
        if !output.ensure(1)? {
            return Ok(NameWalk {
                outcome: Outcome::Truncated {
                    len: 0,
                    unread_rows: 1,
                },
                bytes_len: 0,
                required_bytes: 0,
            });
        };
        let name_token_id = read::<i32>(memory, at(entry.pointer, NAME_TOKEN_ID)?)
            .ok_or("character name token id unreadable")?;
        let name = Literal::read(memory, at(entry.pointer, NAME)?)?;
        let token = Literal::read(memory, at(entry.pointer, TOKEN)?)?;
        Literal::validate(memory, [&name, &token])?;
        let required_bytes = name
            .len
            .checked_add(token.len)
            .ok_or("character text size overflow")?;
        if !bytes.ensure(required_bytes)? {
            return Ok(NameWalk {
                outcome: Outcome::Truncated {
                    len: 0,
                    unread_rows: 1,
                },
                bytes_len: 0,
                required_bytes,
            });
        }
        Literal::copy_all(memory, [&name, &token], &mut bytes[..required_bytes])?;
        let names = required_bytes;
        let template = read::<u64>(memory, at(entry.pointer, TEMPLATE)?)
            .and_then(|pointer| usize::try_from(pointer).ok())
            .ok_or("character template pointer unreadable")?;
        let mut identity = ccharacter_template::Identity::default();
        let mut file_len = 0;
        if template != 0 {
            let mut fits = |len: usize| {
                let required = names
                    .checked_add(len)
                    .ok_or("character text size overflow")?;
                Ok::<_, &'static str>((required, bytes.ensure(required)?))
            };
            let (required, fit) = if let Some(known) = known(template) {
                identity = known.identity;
                file_len = known.file.len();
                let (required, fit) = fits(file_len)?;
                if fit {
                    bytes[names..required].copy_from_slice(known.file);
                }
                (required, fit)
            } else {
                let header = ccharacter_template::read(memory, base, template)?;
                Literal::validate(memory, [&header.file])?;
                identity = header.identity;
                file_len = header.file.len;
                let (required, fit) = fits(file_len)?;
                if fit {
                    Literal::copy_all(memory, [&header.file], &mut bytes[names..required])?;
                }
                (required, fit)
            };
            if !fit {
                return Ok(NameWalk {
                    outcome: Outcome::Truncated {
                        len: 0,
                        unread_rows: 1,
                    },
                    bytes_len: 0,
                    required_bytes: required,
                });
            }
        }
        let required_bytes = names + file_len;
        output[0] = Row {
            id: entry.id,
            country: entry.country,
            name_token_id,
            gender: read::<u32>(memory, at(entry.pointer, GENDER)?)
                .ok_or("character gender unreadable")?,
            roster: entry.roster,
            ordinal: entry.ordinal,
            name_start: 0,
            name_len: name.len,
            token_start: name.len,
            token_len: token.len,
            template,
            template_identity: identity,
            template_file_start: names,
            template_file_len: file_len,
        };
        Ok(NameWalk {
            outcome: Outcome::Rows { len: 1 },
            bytes_len: required_bytes,
            required_bytes,
        })
    })();
    result.unwrap_or_else(|reason| NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub rows: usize,
    pub bytes: usize,
}

pub fn demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<Demand, &'static str> {
    let roster = ccharacter_manager::Roster::read(memory, base, game_state)?;
    let mut cursor = roster.cursor();
    let mut demand = Demand::default();
    while let Some(view) = cursor.next(memory, [true, false, false, false])? {
        let (pointer, _) = view.reference();
        let name = Literal::read(&view, at(pointer, NAME)?)?;
        let token = Literal::read(&view, at(pointer, TOKEN)?)?;
        let template = read::<u64>(&view, at(pointer, TEMPLATE)?)
            .and_then(|template| usize::try_from(template).ok())
            .ok_or("character template pointer unreadable")?;
        let file = if template == 0 {
            0
        } else {
            ccharacter_template::read(memory, base, template)?.file.len
        };
        demand.rows += 1;
        demand.bytes = [name.len, token.len, file]
            .into_iter()
            .try_fold(demand.bytes, usize::checked_add)
            .ok_or("character text size overflow")?;
    }
    Ok(demand)
}

pub fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("character address overflow")
}

pub fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(u32, u32), &'static str> {
    let id = identity_header(memory, base, pointer)?;
    let country =
        read::<u32>(memory, at(pointer, COUNTRY)?).ok_or("character country unreadable")?;
    Ok((id, country))
}

pub fn identity_with_spans(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    offsets: [usize; 3],
    spans: [&mut [u8]; 3],
) -> Result<(u32, usize), &'static str> {
    let mut identity = [0; 16];
    let [first, second, third] = spans;
    let [a, b, c] = offsets.map(|offset| pointer.checked_add(offset).unwrap_or(0));
    let completed = memory.copy_scattered(
        [pointer, a, b, c],
        &mut [&mut identity[..], first, second, third],
    );
    if completed == 0 {
        return Err("character identity unreadable");
    }
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64
        || u32::from_le_bytes(identity[TYPE..ID].try_into().unwrap()) != REFERENCE_TYPE
    {
        return Err("character vtable or reference type mismatch");
    }
    Ok((
        u32::from_le_bytes(identity[ID..16].try_into().unwrap()),
        completed - 1,
    ))
}

pub fn identity_header(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<u32, &'static str> {
    let header = read::<[u8; 16]>(memory, pointer).ok_or("character identity unreadable")?;
    identity_from_header(base, header)
}

pub fn identity_from_header(base: usize, header: [u8; 16]) -> Result<u32, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64
        || u32::from_le_bytes(header[TYPE..ID].try_into().unwrap()) != REFERENCE_TYPE
    {
        return Err("character vtable or reference type mismatch");
    }
    Ok(u32::from_le_bytes(header[ID..16].try_into().unwrap()))
}





pub fn portrait_header(memory: &impl Memory, pointer: usize) -> Result<[u8; 16], &'static str> {
    let country_at = at(pointer, COUNTRY)?;
    let header_at = pointer.checked_add(PORTRAIT_DATA);
    if country_at.checked_add(4).is_some()
        && let Some(header_at) = header_at.filter(|address| address.checked_add(16).is_some())
    {
        let mut country = [0; 4];
        let mut header = [0; 16];
        match memory
            .copy_scattered_prefetched([country_at, header_at], &mut [&mut country, &mut header])
        {
            0 => Err("character country unreadable"),
            2 => Ok(header),
            _ => Err("character portrait header unreadable"),
        }
    } else {
        read::<u32>(memory, country_at).ok_or("character country unreadable")?;
        let address = header_at.ok_or("character portrait address overflow")?;
        read(memory, address).ok_or("character portrait header unreadable")
    }
}
