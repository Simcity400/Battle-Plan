use engine_native_parts::cpolitical_status_layout::{COUNTRY, RULING_PARTY, VTABLE_RVA};

use engine_native_memory::{Memory, cpdx_array::Array, msvc_string, read, walk::Outcome};
use engine_native::{ccharacter, ccountry};
use crate::{ccountry_leader, cpolitical_party};
use engine_native_parts::{cidea, cideology_group};
use engine_sites::Rva;


pub const PARTIES: usize = 0x20;
pub const PARTY_COUNT: usize = 0x2c;
pub const IDEAS: usize = 0x50;
pub const IDEA_COUNT: usize = 0x5c;
pub const TIMED_IDEAS: usize = 0x68;
pub const TIMED_IDEA_STRIDE: usize = 0x18;
pub const TIMED_IDEA_VTABLE_RVA: Rva = Rva(0x029c_cab8);
pub const LAST_ELECTION: usize = 0xa0;
pub const NEXT_ELECTION_CACHE: usize = 0xb8;
pub const RESET_LAST_ELECTION: i32 = 43_800_000 - 365 * 24;
pub fn needs_election_cache(last: i32) -> bool {
    matches!(last, engine_native_memory::cgame_date::NULL_RAW_HOUR | RESET_LAST_ELECTION)
}
pub const ELECTION_FREQUENCY: usize = 0xe8;
pub const ELECTIONS_ALLOWED: usize = 0xec;
const RULING_SPAN: usize = ELECTIONS_ALLOWED + 1 - RULING_PARTY;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub last_election: i32,
    pub election_interval: i32,
    pub next_election_cache: Option<i32>,
    pub capital: i32,
    pub original_capital: i32,
    pub ruling_party: Option<u32>,
    pub has_elections: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IdeaRow {
    pub ordinal: u32,
    pub token_start: usize,
    pub token_len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TimedIdeaRow {
    pub country: u32,
    pub ordinal: u32,
    pub token_start: usize,
    pub token_len: usize,
    pub days: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IdeaDemand {
    pub rows: usize,
    pub bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

impl TokenWalk {
    fn unavailable(reason: &'static str) -> Self {
        Self {
            outcome: Outcome::Unavailable { reason },
            bytes_len: 0,
            required_bytes: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parties {
    pub country: u32,
    pub status: usize,
    pub data: usize,
    pub count: u32,
}

pub trait LeaderCarry {
    fn plan(&mut self, parties: &Parties) -> (usize, Option<usize>);
    fn carry(&mut self, token: usize, rows: &mut [cpolitical_party::LeaderRow]);
    fn party(&mut self, token: usize, party: usize);
    fn character(&mut self, token: usize, character: usize);
    fn read(&mut self, token: usize, rows: &[cpolitical_party::LeaderRow]);
}

pub struct Fresh;
impl LeaderCarry for Fresh {
    fn plan(&mut self, _: &Parties) -> (usize, Option<usize>) {
        (0, None)
    }
    fn carry(&mut self, _: usize, _: &mut [cpolitical_party::LeaderRow]) {}
    fn party(&mut self, _: usize, _: usize) {}
    fn character(&mut self, _: usize, _: usize) {}
    fn read(&mut self, _: usize, _: &[cpolitical_party::LeaderRow]) {}
}

pub trait PartyCarry {
    fn active(&self) -> bool {
        true
    }
    fn plan(&mut self, parties: &Parties) -> (usize, Option<(usize, usize)>);
    fn same_parties(&mut self, token: usize, first: usize, chunk: &[usize]) -> bool;
    fn carry(&mut self, token: usize, rows: &mut [cpolitical_party::Row], text: &mut [u8]);
    fn party(&mut self, token: usize, party: usize);
    fn read(&mut self, token: usize, outcome: Outcome, rows: &[cpolitical_party::Row], text: &[u8]);
}

pub struct FreshParties;
impl PartyCarry for FreshParties {
    fn active(&self) -> bool {
        false
    }
    fn plan(&mut self, _: &Parties) -> (usize, Option<(usize, usize)>) {
        (0, None)
    }
    fn same_parties(&mut self, _: usize, _: usize, _: &[usize]) -> bool {
        false
    }
    fn carry(&mut self, _: usize, _: &mut [cpolitical_party::Row], _: &mut [u8]) {}
    fn party(&mut self, _: usize, _: usize) {}
    fn read(&mut self, _: usize, _: Outcome, _: &[cpolitical_party::Row], _: &[u8]) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ideas {
    pub country: u32,
    pub status: usize,
    pub data: usize,
    pub count: u32,
}

pub trait IdeaCarry {
    fn active(&self) -> bool {
        true
    }
    fn plan(&mut self, ideas: &Ideas) -> (usize, Option<(usize, usize)>);
    fn same_ideas(&mut self, token: usize, first: usize, chunk: &[usize]) -> bool;
    fn carry(&mut self, token: usize, rows: &mut [IdeaRow], text: &mut [u8]);
    fn idea(&mut self, token: usize, slot: usize);
    fn read(&mut self, token: usize, outcome: Outcome, rows: &[IdeaRow], text: &[u8]);
}

pub struct FreshIdeas;
impl IdeaCarry for FreshIdeas {
    fn active(&self) -> bool {
        false
    }
    fn plan(&mut self, _: &Ideas) -> (usize, Option<(usize, usize)>) {
        (0, None)
    }
    fn same_ideas(&mut self, _: usize, _: usize, _: &[usize]) -> bool {
        false
    }
    fn carry(&mut self, _: usize, _: &mut [IdeaRow], _: &mut [u8]) {}
    fn idea(&mut self, _: usize, _: usize) {}
    fn read(&mut self, _: usize, _: Outcome, _: &[IdeaRow], _: &[u8]) {}
}

const POINTER_CHUNK: usize = 32;

fn visit_pointers(
    memory: &impl Memory,
    data: usize,
    count: u32,
    mut visit: impl FnMut(usize, &[usize]) -> bool,
) -> Option<bool> {
    let mut bytes = [0_u8; POINTER_CHUNK * 8];
    let mut pointers = [0_usize; POINTER_CHUNK];
    let mut first = 0_usize;
    while first < count as usize {
        let len = (count as usize - first).min(POINTER_CHUNK);
        let address = data.checked_add(first.checked_mul(8)?)?;
        if !engine_native_memory::read_slice(memory, address, &mut bytes[..len * 8]) {
            return None;
        }
        for (pointer, word) in pointers.iter_mut().zip(bytes[..len * 8].chunks_exact(8)) {
            *pointer = usize::try_from(u64::from_le_bytes(word.try_into().unwrap())).ok()?;
        }
        if !visit(first, &pointers[..len]) {
            return Some(false);
        }
        first += len;
    }
    Some(true)
}

pub struct Reader<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    country: usize,
    status: usize,
}

impl<'a, M: Memory> Reader<'a, M> {
    pub fn new(memory: &'a M, base: usize, country: usize) -> Result<Self, &'static str> {
        let expected =
            engine_sites::at(base, ccountry::VTABLE_RVA).ok_or("module address overflow")?;
        if read::<u64>(memory, country) != Some(expected as u64) {
            return Err("politics country vtable mismatch");
        }
        let status = country
            .checked_add(ccountry::POLITICAL_STATUS)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("political status pointer unreadable or null")?;
        let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
        if read::<u64>(memory, status) != Some(expected as u64) {
            return Err("political status vtable mismatch");
        }
        if status
            .checked_add(COUNTRY)
            .and_then(|p| read::<u64>(memory, p))
            != Some(country as u64)
        {
            return Err("political status owner mismatch");
        }
        Ok(Self {
            memory,
            base,
            country,
            status,
        })
    }

    fn at(&self, offset: usize) -> Result<usize, &'static str> {
        self.status
            .checked_add(offset)
            .ok_or("political status address overflow")
    }

    fn ruling_pointer(&self) -> Result<usize, &'static str> {
        read::<u64>(self.memory, self.at(RULING_PARTY)?)
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("ruling party pointer unreadable")
    }

    pub fn row(&self) -> Result<Row, &'static str> {
        self.row_with_election_cache(true)
    }

    pub fn row_with_election_cache(&self, capture_cache: bool) -> Result<Row, &'static str> {
        let last_election =
            read::<i32>(self.memory, self.at(LAST_ELECTION)?).ok_or("last election unreadable")?;
        let capital_address = self
            .country
            .checked_add(ccountry::CAPITAL)
            .ok_or("current capital unreadable")?;
        let (capitals, ruling) = read_pair::<8, RULING_SPAN>(
            self.memory,
            capital_address,
            self.at(RULING_PARTY),
            [
                "current capital unreadable",
                "ruling party pointer unreadable",
            ],
        )?;
        let capital = i32::from_le_bytes(capitals[..4].try_into().unwrap());
        let original_capital = i32::from_le_bytes(capitals[4..].try_into().unwrap());
        let field = |offset: usize| offset - RULING_PARTY;
        let frequency = i32::from_le_bytes(
            ruling[field(ELECTION_FREQUENCY)..field(ELECTION_FREQUENCY) + 4]
                .try_into()
                .unwrap(),
        );
        let has_elections = ruling[field(ELECTIONS_ALLOWED)] != 0 && frequency > 0;
        let pointer = usize::try_from(u64::from_le_bytes(ruling[..8].try_into().unwrap()))
            .map_err(|_| "ruling party pointer unreadable")?;
        let ruling_party = if pointer == 0 {
            None
        } else {
            Some(engine_native_parts::cpolitical_party_layout::identity(self.memory, self.base, pointer)?.0)
        };
        let next_election_cache =
            (capture_cache && has_elections && needs_election_cache(last_election))
                .then(|| {
                    self.at(NEXT_ELECTION_CACHE)
                        .ok()
                        .and_then(|at| read(self.memory, at))
                })
                .flatten();
        Ok(Row {
            last_election,
            next_election_cache,
            election_interval: frequency,
            capital,
            original_capital,
            ruling_party,
            has_elections,
        })
    }

    pub fn parties(
        &self,
        output: &mut Vec<cpolitical_party::Row>,
        row_start: usize,
        tokens: &mut Vec<u8>,
        byte_start: usize,
    ) -> TokenWalk {
        self.parties_carried(
            0,
            engine_native_memory::walk::Buffer::Growing(output),
            row_start,
            engine_native_memory::walk::Buffer::Growing(tokens),
            byte_start,
            &mut FreshParties,
        )
    }

    pub fn parties_carried(
        &self,
        country: u32,
        mut output: engine_native_memory::walk::Buffer<'_, cpolitical_party::Row>,
        row_start: usize,
        mut tokens: engine_native_memory::walk::Buffer<'_, u8>,
        byte_start: usize,
        carry: &mut impl PartyCarry,
    ) -> TokenWalk {
        let result = (|| {
            let array = Array::read(self.memory, self.at(PARTIES)?, 8)
                .ok_or("political party array header invalid")?;
            let data = array.data;
            let count = array.count;
            output.ensure(
                row_start
                    .checked_add(count as usize)
                    .ok_or("political party row count overflow")?,
            )?;
            let ruling = self.ruling_pointer()?;
            let (token, held) = if carry.active() {
                carry.plan(&Parties {
                    country,
                    status: self.status,
                    data,
                    count,
                })
            } else {
                (0, None)
            };
            if let Some((rows, bytes)) = held
                && rows == count as usize
                && output.len() >= row_start + rows
                && visit_pointers(self.memory, data, count, |first, chunk| {
                    carry.same_parties(token, first, chunk)
                }) == Some(true)
                && tokens.ensure(
                    byte_start
                        .checked_add(bytes)
                        .ok_or("politics token bytes overflow")?,
                )?
            {
                let rows = &mut output[row_start..row_start + rows];
                carry.carry(token, rows, &mut tokens[byte_start..byte_start + bytes]);
                let mut found_ruling = ruling == 0;
                let mut ordinal = 0;
                let fresh = visit_pointers(self.memory, data, count, |_, chunk| {
                    for &pointer in chunk {
                        found_ruling |= pointer == ruling;
                        let Some(popularity) = pointer
                            .checked_add(cpolitical_party::POPULARITY)
                            .and_then(|p| read::<i64>(self.memory, p))
                        else {
                            return false;
                        };
                        rows[ordinal].popularity = popularity;
                        rows[ordinal].is_ruling = u8::from(pointer == ruling);
                        ordinal += 1;
                    }
                    true
                });
                if fresh != Some(true) {
                    return Err("political popularity unreadable");
                }
                if !found_ruling {
                    return Err("ruling party is outside native party roster");
                }
                return Ok(finish(rows.len(), 0, bytes, bytes));
            }
            let mut found_ruling = ruling == 0;
            let mut len = 0_usize;
            let mut bytes_len = 0;
            let mut required_bytes = 0_usize;
            let mut unread_rows = 0;
            for ordinal in 0..count {
                let pointer = read::<u64>(self.memory, data + ordinal as usize * 8)
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .ok_or("political party pointer unreadable or null")?;
                found_ruling |= pointer == ruling;
                carry.party(token, pointer);
                let (id, ideology) = engine_native_parts::cpolitical_party_layout::identity(self.memory, self.base, pointer)?;
                let token = ideology
                    .checked_add(cideology_group::TOKEN)
                    .ok_or("ideology token overflow")?;
                let literal = literal(self.memory, token)?;
                let token_len = literal.len;
                let names = pointer
                    .checked_add(cpolitical_party::TAG)
                    .and_then(|p| read::<[u8; NAMES_SPAN]>(self.memory, p))
                    .ok_or("political party names unreadable")?;
                let field = |offset: usize| offset - cpolitical_party::TAG;
                let tag = u32::from_le_bytes(names[..4].try_into().unwrap());
                let name_default = names[field(cpolitical_party::NAME_DEFAULT)];
                if name_default > 1 {
                    return Err("political party default name flag is not a boolean");
                }
                let text = |offset: usize| {
                    let at = field(offset);
                    literal_from_header(
                        self.memory,
                        pointer + offset,
                        names[at..at + msvc_string::STRIDE].try_into().unwrap(),
                    )
                };
                let name = text(cpolitical_party::NAME)?;
                let long_name = text(cpolitical_party::LONG_NAME)?;
                let row_bytes = token_len
                    .checked_add(name.len)
                    .and_then(|n| n.checked_add(long_name.len))
                    .ok_or("politics token bytes overflow")?;
                required_bytes = required_bytes
                    .checked_add(row_bytes)
                    .ok_or("politics token bytes overflow")?;
                tokens.ensure(
                    byte_start
                        .checked_add(required_bytes)
                        .ok_or("politics token bytes overflow")?,
                )?;
                if row_start + len == output.len()
                    || row_bytes > tokens.len().saturating_sub(byte_start + bytes_len)
                {
                    unread_rows += 1;
                    continue;
                }
                let popularity = pointer
                    .checked_add(cpolitical_party::POPULARITY)
                    .and_then(|p| read::<i64>(self.memory, p))
                    .ok_or("political popularity unreadable")?;
                if output[row_start..row_start + len]
                    .iter()
                    .any(|row| row.id == id)
                {
                    return Err("duplicate native political party id");
                }
                literal
                    .copy(
                        self.memory,
                        &mut tokens[byte_start + bytes_len..byte_start + bytes_len + token_len],
                    )
                    .map_err(|_| "political party token unreadable")?;
                let name_start = bytes_len + token_len;
                name.copy(
                    self.memory,
                    &mut tokens[byte_start + name_start..byte_start + name_start + name.len],
                )
                .map_err(|_| "political party name unreadable")?;
                let long_name_start = name_start + name.len;
                long_name
                    .copy(
                        self.memory,
                        &mut tokens[byte_start + long_name_start
                            ..byte_start + long_name_start + long_name.len],
                    )
                    .map_err(|_| "political party long name unreadable")?;
                output[row_start + len] = cpolitical_party::Row {
                    id,
                    popularity,
                    is_ruling: u8::from(pointer == ruling),
                    token_start: bytes_len,
                    token_len,
                    tag,
                    name_default,
                    name_start,
                    name_len: name.len,
                    long_name_start,
                    long_name_len: long_name.len,
                };
                bytes_len += row_bytes;
                len += 1;
            }
            if !found_ruling {
                return Err("ruling party is outside native party roster");
            }
            let walk = finish(len, unread_rows, bytes_len, required_bytes);
            carry.read(
                token,
                walk.outcome,
                &output[row_start..row_start + len],
                &tokens[byte_start..byte_start + bytes_len],
            );
            Ok(walk)
        })();
        result.unwrap_or_else(TokenWalk::unavailable)
    }

    pub fn party_leaders(
        &self,
        country: u32,
        output: &mut Vec<cpolitical_party::LeaderRow>,
    ) -> Outcome {
        self.party_leaders_buffer(country, engine_native_memory::walk::Buffer::Growing(output))
    }

    pub fn party_leaders_buffer(
        &self,
        country: u32,
        output: engine_native_memory::walk::Buffer<'_, cpolitical_party::LeaderRow>,
    ) -> Outcome {
        self.party_leaders_carried(country, output, &mut Fresh)
    }

    pub fn party_leaders_carried(
        &self,
        country: u32,
        mut output: engine_native_memory::walk::Buffer<'_, cpolitical_party::LeaderRow>,
        carry: &mut impl LeaderCarry,
    ) -> Outcome {
        let result = (|| {
            let parties = Array::read(self.memory, self.at(PARTIES)?, 8)
                .ok_or("political party array header invalid")?;
            let (token, carried) = carry.plan(&Parties {
                country,
                status: self.status,
                data: parties.data,
                count: parties.count,
            });
            if let Some(rows) = carried
                && output.ensure(rows)?
            {
                carry.carry(token, &mut output[..rows]);
                return Ok(Outcome::Rows { len: rows });
            }
            let leader_vtable = engine_sites::at(self.base, engine_native_parts::ccountry_leader_layout::VTABLE_RVA)
                .ok_or("module address overflow")? as u64;
            let mut len = 0_usize;
            let mut unread_rows = 0_u32;
            for index in 0..parties.count {
                let pointer = parties
                    .element(index, 8)
                    .and_then(|slot| read::<u64>(self.memory, slot))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .ok_or("political party pointer unreadable or null")?;
                carry.party(token, pointer);
                let (party, _) = engine_native_parts::cpolitical_party_layout::identity(self.memory, self.base, pointer)?;
                let leaders = pointer
                    .checked_add(engine_native_parts::cpolitical_party_layout::COUNTRY_LEADERS)
                    .and_then(|address| Array::read(self.memory, address, 8))
                    .ok_or("country leader vector header invalid")?;
                output.ensure(
                    len.checked_add(leaders.count as usize)
                        .ok_or("country leader row demand overflow")?,
                )?;
                for ordinal in 0..leaders.count {
                    if len == output.len() {
                        unread_rows = unread_rows
                            .checked_add(leaders.count - ordinal)
                            .ok_or("country leader count overflow")?;
                        break;
                    }
                    let leader = leaders
                        .element(ordinal, 8)
                        .and_then(|slot| read::<u64>(self.memory, slot))
                        .and_then(|p| usize::try_from(p).ok())
                        .filter(|&p| p != 0)
                        .ok_or("country leader pointer unreadable or null")?;
                    let header =
                        read::<[u8; 16]>(self.memory, leader).ok_or("country leader unreadable")?;
                    if u64::from_le_bytes(header[..8].try_into().unwrap()) != leader_vtable {
                        return Err("country leader vtable mismatch");
                    }
                    let parent = usize::try_from(u64::from_le_bytes(
                        header[engine_native_parts::ccountry_leader_layout::CHARACTER..engine_native_parts::ccountry_leader_layout::CHARACTER + 8]
                            .try_into()
                            .unwrap(),
                    ))
                    .map_err(|_| "country leader character pointer overflow")?;
                    let character = if parent == 0 {
                        None
                    } else {
                        carry.character(token, parent);
                        Some(ccharacter::identity_header(self.memory, self.base, parent)?)
                    };
                    let role = character
                        .map(|_| ccountry_leader::role_key(self.memory, parent, leader))
                        .transpose()?;
                    let leader_id = leader
                        .checked_add(ccountry_leader::LEADER_ID)
                        .and_then(|address| read::<i32>(self.memory, address))
                        .ok_or("country leader id unreadable")?;
                    output[len] = cpolitical_party::LeaderRow {
                        country,
                        party,
                        ordinal,
                        character,
                        role,
                        leader_id,
                    };
                    len += 1;
                }
            }
            if unread_rows == 0 {
                carry.read(token, &output[..len]);
            }
            Ok(if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            })
        })();
        result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
    }

    pub fn ideas(&self, output: &mut Vec<IdeaRow>, tokens: &mut Vec<u8>) -> TokenWalk {
        self.ideas_buffer(
            engine_native_memory::walk::Buffer::Growing(output),
            engine_native_memory::walk::Buffer::Growing(tokens),
        )
    }

    pub fn ideas_buffer(
        &self,
        output: engine_native_memory::walk::Buffer<'_, IdeaRow>,
        tokens: engine_native_memory::walk::Buffer<'_, u8>,
    ) -> TokenWalk {
        self.ideas_carried(0, output, tokens, &mut FreshIdeas)
    }

    pub fn ideas_carried(
        &self,
        country: u32,
        mut output: engine_native_memory::walk::Buffer<'_, IdeaRow>,
        mut tokens: engine_native_memory::walk::Buffer<'_, u8>,
        carry: &mut impl IdeaCarry,
    ) -> TokenWalk {
        let result = (|| {
            let array =
                Array::read(self.memory, self.at(IDEAS)?, 8).ok_or("active idea array invalid")?;
            let (token, held) = if carry.active() {
                carry.plan(&Ideas {
                    country,
                    status: self.status,
                    data: array.data,
                    count: array.count,
                })
            } else {
                (0, None)
            };
            if let Some((rows, bytes)) = held
                && rows == array.count as usize
                && output.ensure(rows)?
                && tokens.ensure(bytes)?
                && visit_pointers(self.memory, array.data, array.count, |first, chunk| {
                    carry.same_ideas(token, first, chunk)
                }) == Some(true)
            {
                carry.carry(token, &mut output[..rows], &mut tokens[..bytes]);
                return Ok(finish(rows, 0, bytes, bytes));
            }
            let mut len = 0_usize;
            let mut bytes_len = 0;
            let mut required_bytes = 0_usize;
            let mut unread_rows = 0;
            output.ensure(array.count as usize)?;
            let primary_vtable =
                engine_sites::at(self.base, cidea::VTABLE_RVA).ok_or("module address overflow")?;
            let secondary_vtable = engine_sites::at(self.base, cidea::SECONDARY_VTABLE_RVA)
                .ok_or("module address overflow")?;
            for ordinal in 0..array.count {
                let slot = array
                    .element(ordinal, 8)
                    .ok_or("active idea array overflow")?;
                let mut pointer = read::<u64>(self.memory, slot)
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .ok_or("active idea pointer unreadable or null")?;
                carry.idea(token, pointer);
                let vtable =
                    read::<u64>(self.memory, pointer).ok_or("active idea vtable unreadable")?;
                if vtable == secondary_vtable as u64 {
                    pointer = pointer
                        .checked_sub(cidea::SECONDARY_BASE)
                        .ok_or("active idea primary underflow")?;
                } else if vtable != primary_vtable as u64 {
                    return Err("active idea vtable mismatch");
                }
                if read::<u64>(self.memory, pointer) != Some(primary_vtable as u64) {
                    return Err("active idea primary vtable mismatch");
                }
                let token = pointer
                    .checked_add(cidea::TOKEN)
                    .ok_or("active idea token overflow")?;
                let literal = literal(self.memory, token)?;
                let token_len = literal.len;
                required_bytes = required_bytes
                    .checked_add(token_len)
                    .ok_or("politics token bytes overflow")?;
                tokens.ensure(required_bytes)?;
                if len == output.len() || token_len > tokens.len() - bytes_len {
                    unread_rows += 1;
                    continue;
                }
                literal
                    .copy(self.memory, &mut tokens[bytes_len..bytes_len + token_len])
                    .map_err(|_| "active idea token unreadable")?;
                output[len] = IdeaRow {
                    ordinal,
                    token_start: bytes_len,
                    token_len,
                };
                bytes_len += token_len;
                len += 1;
            }
            let walk = finish(len, unread_rows, bytes_len, required_bytes);
            carry.read(token, walk.outcome, &output[..len], &tokens[..bytes_len]);
            Ok(walk)
        })();
        result.unwrap_or_else(TokenWalk::unavailable)
    }

    pub fn timed_ideas(
        &self,
        country: u32,
        output: &mut Vec<TimedIdeaRow>,
        tokens: &mut Vec<u8>,
    ) -> TokenWalk {
        self.timed_ideas_buffer(
            country,
            engine_native_memory::walk::Buffer::Growing(output),
            engine_native_memory::walk::Buffer::Growing(tokens),
        )
    }

    pub fn timed_ideas_buffer(
        &self,
        country: u32,
        mut output: engine_native_memory::walk::Buffer<'_, TimedIdeaRow>,
        mut tokens: engine_native_memory::walk::Buffer<'_, u8>,
    ) -> TokenWalk {
        let result = (|| {
            let array = Array::read(self.memory, self.at(TIMED_IDEAS)?, TIMED_IDEA_STRIDE)
                .ok_or("timed idea array unreadable or invalid")?;
            let mut len = 0_usize;
            let mut bytes_len = 0;
            let mut required_bytes = 0_usize;
            let mut unread_rows = 0_u32;
            output.ensure(array.count as usize)?;
            for ordinal in 0..array.count {
                let address = array
                    .element(ordinal, TIMED_IDEA_STRIDE)
                    .ok_or("timed idea entry address overflow")?;
                let entry = read::<[u8; TIMED_IDEA_STRIDE]>(self.memory, address)
                    .ok_or("timed idea entry unreadable")?;
                if engine_sites::at(self.base, TIMED_IDEA_VTABLE_RVA).map(|p| p as u64)
                    != Some(u64::from_le_bytes(entry[..8].try_into().unwrap()))
                {
                    return Err("timed idea entry vtable mismatch");
                }
                let pointer = usize::try_from(u64::from_le_bytes(entry[8..16].try_into().unwrap()))
                    .map_err(|_| "timed idea definition address overflow")?;
                if engine_sites::at(self.base, cidea::VTABLE_RVA).map(|p| p as u64)
                    != read::<u64>(self.memory, pointer)
                {
                    return Err("timed idea definition vtable mismatch");
                }
                let token = literal(
                    self.memory,
                    pointer
                        .checked_add(cidea::TOKEN)
                        .ok_or("timed idea token address overflow")?,
                )?;
                required_bytes = required_bytes
                    .checked_add(token.len)
                    .ok_or("timed idea token demand overflow")?;
                tokens.ensure(required_bytes)?;
                if len == output.len() || token.len > tokens.len() - bytes_len {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("timed idea omitted count overflow")?;
                    continue;
                }
                token
                    .copy(self.memory, &mut tokens[bytes_len..bytes_len + token.len])
                    .map_err(|_| "timed idea token bytes unreadable")?;
                output[len] = TimedIdeaRow {
                    country,
                    ordinal,
                    token_start: bytes_len,
                    token_len: token.len,
                    days: i32::from_le_bytes(entry[16..20].try_into().unwrap()),
                };
                len += 1;
                bytes_len += token.len;
            }
            Ok(finish(len, unread_rows, bytes_len, required_bytes))
        })();
        result.unwrap_or_else(TokenWalk::unavailable)
    }
}

pub fn count_timed_ideas(memory: &impl Memory, base: usize, state: usize) -> TokenWalk {
    walk_timed_ideas_buffer(memory, base, state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count)
}

pub fn walk_timed_ideas(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<TimedIdeaRow>,
    tokens: &mut Vec<u8>,
) -> TokenWalk {
    walk_timed_ideas_buffer(
        memory,
        base,
        state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(tokens),
    )
}

pub fn walk_timed_ideas_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, TimedIdeaRow>,
    mut tokens: engine_native_memory::walk::Buffer<'_, u8>,
) -> TokenWalk {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, state)?;
        let mut len = 0_usize;
        let mut bytes_len = 0;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0_u32;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let reader = Reader::new(memory, base, country.pointer)?;
            let walk =
                reader.timed_ideas_buffer(country.id, output.suffix(len), tokens.suffix(bytes_len));
            if let Outcome::Unavailable { reason } = walk.outcome {
                return Err(reason);
            }
            for row in &mut output[len..len + walk.outcome.len()] {
                row.token_start += bytes_len;
            }
            len += walk.outcome.len();
            bytes_len += walk.bytes_len;
            required_bytes = required_bytes
                .checked_add(walk.required_bytes)
                .ok_or("timed idea token demand overflow")?;
            if let Outcome::Truncated {
                unread_rows: count, ..
            } = walk.outcome
            {
                unread_rows = unread_rows
                    .checked_add(count)
                    .ok_or("timed idea omitted count overflow")?;
            }
        }
        Ok(finish(len, unread_rows, bytes_len, required_bytes))
    })();
    result.unwrap_or_else(TokenWalk::unavailable)
}

pub fn timed_idea_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<IdeaDemand, &'static str> {
    let walk = count_timed_ideas(memory, base, state);
    match walk.outcome {
        Outcome::Unavailable { reason } => Err(reason),
        Outcome::Rows { len } => Ok(IdeaDemand {
            rows: len,
            bytes: walk.required_bytes,
        }),
        Outcome::Truncated { len, unread_rows } => Ok(IdeaDemand {
            rows: len
                .checked_add(unread_rows as usize)
                .ok_or("timed idea row demand overflow")?,
            bytes: walk.required_bytes,
        }),
    }
}

#[inline(always)]
fn read_pair<const A: usize, const B: usize>(
    memory: &impl Memory,
    first: usize,
    second: Result<usize, &'static str>,
    unreadable: [&'static str; 2],
) -> Result<([u8; A], [u8; B]), &'static str> {
    let second = match second {
        Ok(address) => address,
        Err(reason) => {
            read::<[u8; A]>(memory, first).ok_or(unreadable[0])?;
            return Err(reason);
        }
    };
    let mut first_bytes = [0; A];
    let mut second_bytes = [0; B];
    match memory.copy_scattered([first, second], &mut [&mut first_bytes, &mut second_bytes]) {
        0 => Err(unreadable[0]),
        2 => Ok((first_bytes, second_bytes)),
        _ => Err(unreadable[1]),
    }
}

fn finish(len: usize, unread_rows: u32, bytes_len: usize, required_bytes: usize) -> TokenWalk {
    TokenWalk {
        outcome: if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        },
        bytes_len,
        required_bytes,
    }
}

const NAMES_SPAN: usize = cpolitical_party::NAME_DEFAULT + 1 - cpolitical_party::TAG;

pub fn party_leader_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut total = 0_usize;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let Ok(reader) = Reader::new(memory, base, country.pointer) else {
            continue;
        };
        if let Outcome::Truncated { unread_rows, .. } = reader.party_leaders_buffer(country.id, engine_native_memory::walk::Buffer::Count) {
            total = total
                .checked_add(unread_rows as usize)
                .ok_or("party leader total overflow")?;
        }
    }
    Ok(total)
}

fn literal(memory: &impl Memory, address: usize) -> Result<msvc_string::Descriptor, &'static str> {
    msvc_string::Descriptor::read(memory, address).map_err(literal_error)
}

fn literal_from_header(
    memory: &impl Memory,
    address: usize,
    header: [u8; msvc_string::STRIDE],
) -> Result<msvc_string::Descriptor, &'static str> {
    msvc_string::Descriptor::from_header(memory, address, header).map_err(literal_error)
}

fn literal_error(reason: &'static str) -> &'static str {
    match reason {
        "string header unreadable" => "politics string header unreadable",
        "string header invalid" => "politics string header invalid",
        "string length overflow" => "politics string length overflow",
        "string heap pointer invalid" => "politics string heap pointer null or overflow",
        _ => "politics string terminator unreadable or invalid",
    }
}

#[cfg(test)]
mod pair_tests;

#[cfg(test)]
mod tests;
