//! Current politics, complete native party roster and active idea identities.
//!
//! Each party's popularity and naming operands (tag, name and long-name keys,
//! default-name flag) share one read; `cpolitical_party` owns their meaning.
//! Synthetic fixtures cover renamed/default parties, a non-boolean flag and
//! truncation; live capture is unverified.

use crate::{
    Memory, ccharacter, ccountry, ccountry_leader, cidea, cideology_group, cpdx_array::Array,
    cpolitical_party, msvc_string, read, walk::Outcome,
};

/// Political-status primary vtable.
pub const VTABLE_RVA: usize = 0x0293_9768;
/// Owning-country pointer authenticated by backlink.
pub const COUNTRY: usize = 0xc8;
/// Political power, signed i64/100000, accumulated and clamped as a full qword.
pub const POLITICAL_POWER: usize = 0xe0;

/// Party-pointer array.
pub const PARTIES: usize = 0x20;
/// Live party count, signed i32.
pub const PARTY_COUNT: usize = 0x2c;
/// CPoliticalStatus party-array capacity, signed i32; standard CPdxArray header paired with native +0x2c count.
pub const PARTY_CAPACITY: usize = 0x28;
/// Active idea-pointer array.
pub const IDEAS: usize = 0x50;
/// Idea-array capacity, u32.
pub const IDEA_CAPACITY: usize = 0x58;
/// Idea-array count, u32.
pub const IDEA_COUNT: usize = 0x5c;
/// Stored daily entry list; each signed counter is decremented and then
/// swap-removed when expired.
pub const TIMED_IDEAS: usize = 0x68;
pub const TIMED_IDEA_STRIDE: usize = 0x18;
/// Timed-idea primary vtable for roster entries.
pub const TIMED_IDEA_VTABLE_RVA: usize = 0x0293_9718;
/// Last-election raw hour, i32, written by the election and daily update paths.
pub const LAST_ELECTION: usize = 0xa0;
/// Ruling-party pointer resolved from serialized party IDs.
pub const RULING_PARTY: usize = 0xd0;
/// Election frequency, i32, initialized to 48.
pub const ELECTION_FREQUENCY: usize = 0xe8;
/// Elections-allowed boolean byte.
pub const ELECTIONS_ALLOWED: usize = 0xec;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub last_election: i32,
    pub election_frequency: i32,
    pub elections_allowed: u8,
    pub capital: i32,
    /// Alternate-capital raw reference preferred unless zero.
    pub original_capital: i32,
    /// None is the engine's null pointer, not a failed ruling-party read.
    pub ruling_party: Option<u32>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IdeaRow {
    /// Native roster position; the literal token is the definition's identity.
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
    /// Stored remaining days, separate from an expiry date.
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

/// Authenticates the country/status ownership once for this callback. Individual
/// scalar, party and idea reads retain independent failures and capacities.
pub struct Reader<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    country: usize,
    status: usize,
}

impl<'a, M: Memory> Reader<'a, M> {
    pub fn new(memory: &'a M, base: usize, country: usize) -> Result<Self, &'static str> {
        let expected = base
            .checked_add(ccountry::VTABLE_RVA)
            .ok_or("module address overflow")?;
        if read::<u64>(memory, country) != Some(expected as u64) {
            return Err("politics country vtable mismatch");
        }
        let status = country
            .checked_add(ccountry::POLITICAL_STATUS)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("political status pointer unreadable or null")?;
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
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
        // Frequency and the flag are adjacent, exact fields (five bytes).
        let (last_election, fields) = read_pair::<4, 5>(
            self.memory,
            self.at(LAST_ELECTION)?,
            self.at(ELECTION_FREQUENCY),
            ["last election unreadable", "election fields unreadable"],
        )?;
        let last_election = i32::from_le_bytes(last_election);
        let election_frequency = i32::from_le_bytes(fields[..4].try_into().unwrap());
        let elections_allowed = fields[4];
        if elections_allowed > 1 {
            return Err("elections allowed is not a boolean");
        }
        let capital_address = self
            .country
            .checked_add(ccountry::CAPITAL)
            .ok_or("current capital unreadable")?;
        let (capitals, pointer) = read_pair::<8, 8>(
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
        let pointer = usize::try_from(u64::from_le_bytes(pointer))
            .map_err(|_| "ruling party pointer unreadable")?;
        let ruling_party = if pointer == 0 {
            None
        } else {
            Some(cpolitical_party::identity(self.memory, self.base, pointer)?.0)
        };
        Ok(Row {
            last_election,
            election_frequency,
            elections_allowed,
            capital,
            original_capital,
            ruling_party,
        })
    }

    pub fn parties(&self, output: &mut [cpolitical_party::Row], tokens: &mut [u8]) -> TokenWalk {
        self.parties_into(
            crate::walk::Buffer::Fixed(output),
            0,
            crate::walk::Buffer::Fixed(tokens),
            0,
        )
    }

    /// Worker-only continuation from this capture's original party header and
    /// each acquired string length. The preceding countries' rows stay owned.
    pub fn parties_growing(
        &self,
        output: &mut Vec<cpolitical_party::Row>,
        row_start: usize,
        tokens: &mut Vec<u8>,
        byte_start: usize,
    ) -> TokenWalk {
        self.parties_into(
            crate::walk::Buffer::Growing(output),
            row_start,
            crate::walk::Buffer::Growing(tokens),
            byte_start,
        )
    }

    fn parties_into(
        &self,
        mut output: crate::walk::Buffer<'_, cpolitical_party::Row>,
        row_start: usize,
        mut tokens: crate::walk::Buffer<'_, u8>,
        byte_start: usize,
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
                let (id, ideology) = cpolitical_party::identity(self.memory, self.base, pointer)?;
                let token = ideology
                    .checked_add(cideology_group::TOKEN)
                    .ok_or("ideology token overflow")?;
                let literal = literal(self.memory, token)?;
                let token_len = literal.len;
                // Tag, both name keys and the default flag are adjacent: one read.
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
            Ok(finish(len, unread_rows, bytes_len, required_bytes))
        })();
        result.unwrap_or_else(TokenWalk::unavailable)
    }

    /// Every party's country leaders (PartyLeaders): the party array and each
    /// leader vector retain source order. Authenticate each leader vtable. A null
    /// parent character supplies `None`, matching the serialized invalid reference;
    /// a present parent must authenticate as a character. Once a row does not fit,
    /// count later leaders from vector headers only. Empty output reports the
    /// country's total as `unread_rows`.
    pub fn party_leaders(
        &self,
        country: u32,
        output: &mut [cpolitical_party::LeaderRow],
    ) -> Outcome {
        self.party_leaders_buffer(country, crate::walk::Buffer::Fixed(output))
    }

    pub fn party_leaders_growing(
        &self,
        country: u32,
        output: &mut Vec<cpolitical_party::LeaderRow>,
    ) -> Outcome {
        self.party_leaders_buffer(country, crate::walk::Buffer::Growing(output))
    }

    pub fn party_leaders_buffer(
        &self,
        country: u32,
        mut output: crate::walk::Buffer<'_, cpolitical_party::LeaderRow>,
    ) -> Outcome {
        let result = (|| {
            let parties = Array::read(self.memory, self.at(PARTIES)?, 8)
                .ok_or("political party array header invalid")?;
            let leader_vtable = self
                .base
                .checked_add(ccountry_leader::VTABLE_RVA)
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
                let (party, _) = cpolitical_party::identity(self.memory, self.base, pointer)?;
                let leaders = pointer
                    .checked_add(cpolitical_party::COUNTRY_LEADERS)
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
                        header[ccountry_leader::CHARACTER..ccountry_leader::CHARACTER + 8]
                            .try_into()
                            .unwrap(),
                    ))
                    .map_err(|_| "country leader character pointer overflow")?;
                    let character = if parent == 0 {
                        None
                    } else {
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
            Ok(if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            })
        })();
        result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
    }

    /// Captures exact active CIdea definition tokens. Laws, spirits and advisor
    /// roles are later joins with verified content, not inferred memory fields.
    pub fn ideas(&self, output: &mut [IdeaRow], tokens: &mut [u8]) -> TokenWalk {
        self.ideas_buffer(
            crate::walk::Buffer::Fixed(output),
            crate::walk::Buffer::Fixed(tokens),
        )
    }

    pub fn ideas_growing(&self, output: &mut Vec<IdeaRow>, tokens: &mut Vec<u8>) -> TokenWalk {
        self.ideas_buffer(
            crate::walk::Buffer::Growing(output),
            crate::walk::Buffer::Growing(tokens),
        )
    }

    pub fn ideas_buffer(
        &self,
        mut output: crate::walk::Buffer<'_, IdeaRow>,
        mut tokens: crate::walk::Buffer<'_, u8>,
    ) -> TokenWalk {
        let result = (|| {
            let array =
                Array::read(self.memory, self.at(IDEAS)?, 8).ok_or("active idea array invalid")?;
            let mut len = 0_usize;
            let mut bytes_len = 0;
            let mut required_bytes = 0_usize;
            let mut unread_rows = 0;
            output.ensure(array.count as usize)?;
            let primary_vtable = self
                .base
                .checked_add(cidea::VTABLE_RVA)
                .ok_or("module address overflow")?;
            let secondary_vtable = self
                .base
                .checked_add(cidea::SECONDARY_VTABLE_RVA)
                .ok_or("module address overflow")?;
            for ordinal in 0..array.count {
                let slot = array
                    .element(ordinal, 8)
                    .ok_or("active idea array overflow")?;
                let mut pointer = read::<u64>(self.memory, slot)
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .ok_or("active idea pointer unreadable or null")?;
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
            Ok(finish(len, unread_rows, bytes_len, required_bytes))
        })();
        result.unwrap_or_else(TokenWalk::unavailable)
    }

    pub fn timed_ideas(
        &self,
        country: u32,
        output: &mut [TimedIdeaRow],
        tokens: &mut [u8],
    ) -> TokenWalk {
        self.timed_ideas_buffer(
            country,
            crate::walk::Buffer::Fixed(output),
            crate::walk::Buffer::Fixed(tokens),
        )
    }

    pub fn timed_ideas_growing(
        &self,
        country: u32,
        output: &mut Vec<TimedIdeaRow>,
        tokens: &mut Vec<u8>,
    ) -> TokenWalk {
        self.timed_ideas_buffer(
            country,
            crate::walk::Buffer::Growing(output),
            crate::walk::Buffer::Growing(tokens),
        )
    }

    pub fn timed_ideas_buffer(
        &self,
        country: u32,
        mut output: crate::walk::Buffer<'_, TimedIdeaRow>,
        mut tokens: crate::walk::Buffer<'_, u8>,
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
                if self
                    .base
                    .checked_add(TIMED_IDEA_VTABLE_RVA)
                    .map(|p| p as u64)
                    != Some(u64::from_le_bytes(entry[..8].try_into().unwrap()))
                {
                    return Err("timed idea entry vtable mismatch");
                }
                let pointer = usize::try_from(u64::from_le_bytes(entry[8..16].try_into().unwrap()))
                    .map_err(|_| "timed idea definition address overflow")?;
                if self.base.checked_add(cidea::VTABLE_RVA).map(|p| p as u64)
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

/// Exact token lengths are counted even when caller storage is short.
pub fn walk_timed_ideas(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut [TimedIdeaRow],
    tokens: &mut [u8],
) -> TokenWalk {
    walk_timed_ideas_buffer(
        memory,
        base,
        state,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(tokens),
    )
}

pub fn walk_timed_ideas_growing(
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
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(tokens),
    )
}

pub fn walk_timed_ideas_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: crate::walk::Buffer<'_, TimedIdeaRow>,
    mut tokens: crate::walk::Buffer<'_, u8>,
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
    let walk = walk_timed_ideas(memory, base, state, &mut [], &mut []);
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

/// Exact ordered fields with no value-dependent branch between them. If the
/// later address overflows, retain the preceding scalar observation and its
/// first-fault error instead of rejecting the whole pair before that read.
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

/// A party's tag through its default-name flag, inclusive.
const NAMES_SPAN: usize = cpolitical_party::NAME_DEFAULT + 1 - cpolitical_party::TAG;

/// Every country's party leader total, from the party and leader vector
/// headers (`Reader::party_leaders` with no row storage), for sizing. A
/// country whose political status does not authenticate adds nothing: the
/// hourly walk reports it.
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
        if let Outcome::Truncated { unread_rows, .. } = reader.party_leaders(country.id, &mut []) {
            total = total
                .checked_add(unread_rows as usize)
                .ok_or("party leader total overflow")?;
        }
    }
    Ok(total)
}

fn literal(memory: &impl Memory, address: usize) -> Result<msvc_string::Descriptor, &'static str> {
    // Inline tokens and their terminators are already in the guarded header.
    // Heap payloads retain their separate, capacity-gated acquisition.
    msvc_string::Descriptor::read(memory, address).map_err(literal_error)
}

/// [`literal`] over a header already copied from `address`.
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
