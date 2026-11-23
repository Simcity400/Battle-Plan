
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use engine_native::{ccountry, cgame_state};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x0279_f740);
pub const ID_TOKEN: usize = 0x08;
pub const BASE_BONUS: usize = 0x10;
pub const MEMBERS: usize = 0x18;
pub const MEMBER_CAPACITY: usize = 0x20;
pub const MEMBER_COUNT: usize = 0x24;
pub const BONUSES: usize = 0x30;
pub const BONUS_COUNT: usize = 0x3c;
pub const TEMPLATE: usize = 0x48;
pub const MEMBER_STRIDE: usize = 4;
pub const BONUS_STRIDE: usize = 8;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub base_bonus: i64,
    pub member_count: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemberRow {
    pub group: u32,
    pub ordinal: u32,
    pub country: u32,
    pub bonus: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub groups: Outcome,
    pub members: Outcome,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Walk {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    groups: &mut Vec<Row>,
    members: &mut Vec<MemberRow>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(groups),
        engine_native_memory::walk::Buffer::Growing(members),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut groups: engine_native_memory::walk::Buffer<'_, Row>,
    mut members: engine_native_memory::walk::Buffer<'_, MemberRow>,
) -> Walk {
    let result = (|| {
        let array = game_state
            .checked_add(cgame_state::TECH_SHARING_GROUPS)
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("technology-sharing group array unreadable or invalid")?;
        if array.count == 0 {
            return Ok(Walk {
                groups: Outcome::Rows { len: 0 },
                members: Outcome::Rows { len: 0 },
            });
        }
        groups.ensure(array.count as usize)?;
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut group_len = 0;
        let mut member_len = 0_usize;
        let mut group_omitted = 0_u32;
        let mut member_omitted = 0_u32;
        for index in 0..array.count {
            let pointer = array
                .element(index, 8)
                .and_then(|p| read::<u64>(memory, p))
                .and_then(|p| usize::try_from(p).ok())
                .filter(|&p| p != 0)
                .ok_or("technology-sharing group pointer unreadable or null")?;
            let header = header(memory, base, pointer)?;
            if groups[..group_len].iter().any(|row| row.id == header.id) {
                return Err("duplicate technology-sharing group identity");
            }
            if group_len < groups.len() {
                groups[group_len] = Row {
                    id: header.id,
                    base_bonus: header.base_bonus,
                    member_count: header.members.count,
                };
                group_len += 1;
            } else {
                group_omitted += 1;
            }
            members.ensure(
                member_len
                    .checked_add(header.members.count as usize)
                    .ok_or("technology-sharing member demand overflow")?,
            )?;
            for ordinal in 0..header.members.count {
                if member_len == members.len() {
                    member_omitted = member_omitted
                        .checked_add(header.members.count - ordinal)
                        .ok_or("technology-sharing omitted row count overflow")?;
                    break;
                }
                let country = header
                    .members
                    .element(ordinal, MEMBER_STRIDE)
                    .and_then(|p| read::<u32>(memory, p))
                    .ok_or("technology-sharing member unreadable")?;
                countries.resolve(country)?;
                let bonus = header
                    .bonuses
                    .element(ordinal, BONUS_STRIDE)
                    .and_then(|p| read::<i64>(memory, p))
                    .ok_or("technology-sharing member bonus unreadable")?;
                members[member_len] = MemberRow {
                    group: header.id,
                    ordinal,
                    country,
                    bonus,
                };
                member_len += 1;
            }
        }
        Ok(Walk {
            groups: outcome(group_len, group_omitted),
            members: outcome(member_len, member_omitted),
        })
    })();
    result.unwrap_or_else(|reason| Walk {
        groups: Outcome::Unavailable { reason },
        members: Outcome::Unavailable { reason },
    })
}

struct Header {
    id: u32,
    base_bonus: i64,
    members: Array,
    bonuses: Array,
}

fn header(memory: &impl Memory, base: usize, pointer: usize) -> Result<Header, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("technology-sharing group vtable mismatch");
    }
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("technology-sharing address overflow")
    };
    let id = read::<u32>(memory, at(ID_TOKEN)?)
        .filter(|&id| id != 0 && id <= i32::MAX as u32)
        .ok_or("technology-sharing group identity unreadable or invalid")?;
    let base_bonus =
        read::<i64>(memory, at(BASE_BONUS)?).ok_or("technology-sharing base bonus unreadable")?;
    let members = Array::read(memory, at(MEMBERS)?, MEMBER_STRIDE)
        .ok_or("technology-sharing member array unreadable or invalid")?;
    let bonuses = Array::read(memory, at(BONUSES)?, BONUS_STRIDE)
        .ok_or("technology-sharing bonus array unreadable or invalid")?;
    if members.count != bonuses.count {
        return Err("technology-sharing member and bonus counts differ");
    }
    Ok(Header {
        id,
        base_bonus,
        members,
        bonuses,
    })
}

fn outcome(len: usize, unread_rows: u32) -> Outcome {
    if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const BASE: usize = 0x1000_0000;
    const GAME: usize = 0x1000;
    const GROUP: usize = 0x5000;

    #[derive(Default)]
    struct MemoryImage(BTreeMap<usize, u8>);

    impl MemoryImage {
        fn put(&mut self, address: usize, bytes: &[u8]) {
            for (offset, &value) in bytes.iter().enumerate() {
                self.0.insert(address + offset, value);
            }
        }
        fn array(&mut self, address: usize, data: u64, count: u32) {
            self.put(address, &data.to_le_bytes());
            self.put(address + 8, &count.to_le_bytes());
            self.put(address + 12, &count.to_le_bytes());
        }
    }

    impl Memory for MemoryImage {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            for (offset, byte) in output.iter_mut().enumerate() {
                let Some(&value) = self.0.get(&(address + offset)) else {
                    return false;
                };
                *byte = value;
            }
            true
        }
    }

    fn fixture() -> MemoryImage {
        let mut memory = MemoryImage::default();
        memory.array(GAME + cgame_state::TECH_SHARING_GROUPS, 0x2000, 1);
        memory.put(0x2000, &(GROUP as u64).to_le_bytes());
        memory.put(GROUP, &((BASE + VTABLE_RVA.0) as u64).to_le_bytes());
        memory.put(GROUP + ID_TOKEN, &1_u32.to_le_bytes());
        memory.put(GROUP + BASE_BONUS, &(-7_000_000_001_i64).to_le_bytes());
        memory.array(GROUP + MEMBERS, 0x6000, 2);
        memory.array(GROUP + BONUSES, 0x6100, 2);
        memory.put(0x6000, &1_u32.to_le_bytes());
        memory.put(0x6004, &3_u32.to_le_bytes());
        memory.put(0x6100, &i64::MIN.to_le_bytes());
        memory.put(0x6108, &9_000_000_007_i64.to_le_bytes());
        memory.array(GAME + cgame_state::COUNTRIES, 0x3000, 3);
        memory.array(GAME + cgame_state::COUNTRY_LINKS, 0x3200, 4);
        for (index, pointer) in [0_u64, 0x4000, 0x4100].into_iter().enumerate() {
            memory.put(0x3000 + index * 8, &pointer.to_le_bytes());
        }
        for (index, canonical) in [0_u32, 1, 2, 2].into_iter().enumerate() {
            memory.put(0x3200 + index * 4, &canonical.to_le_bytes());
        }
        for (pointer, id) in [(0x4000, 1_u32), (0x4100, 2)] {
            memory.put(
                pointer,
                &((BASE + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            memory.put(pointer + ccountry::ID, &id.to_le_bytes());
        }
        memory
    }

    #[test]
    fn paired_members_preserve_alias_handles_native_order_and_signed_precision() {
        let memory = fixture();
        let mut groups = vec![Row::default(); 1];
        let mut members = vec![MemberRow::default(); 2];
        let result = walk(&memory, BASE, GAME, &mut groups, &mut members);
        assert_eq!(result.groups, Outcome::Rows { len: 1 });
        assert_eq!(result.members, Outcome::Rows { len: 2 });
        assert_eq!(groups[0].base_bonus, -7_000_000_001);
        assert_eq!(groups[0].id, 1);
        assert_eq!(
            members[0],
            MemberRow {
                group: 1,
                ordinal: 0,
                country: 1,
                bonus: i64::MIN
            }
        );
        assert_eq!(
            members[1],
            MemberRow {
                group: 1,
                ordinal: 1,
                country: 3,
                bonus: 9_000_000_007
            }
        );
    }

    #[test]
    fn the_count_probe_counts_each_table_without_the_lexer() {
        let memory = fixture();
        let result = count(&memory, BASE, GAME);
        assert_eq!(
            result.groups,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!(
            result.members,
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
    }

    #[test]
    fn mismatched_pairs_invalid_country_and_negative_counts_are_unavailable() {
        let mut memory = fixture();
        let mut groups = vec![Row::default(); 1];
        let mut members = vec![MemberRow::default(); 2];
        memory.put(GROUP + BONUS_COUNT, &1_u32.to_le_bytes());
        assert!(matches!(
            walk(&memory, BASE, GAME, &mut groups, &mut members).members,
            Outcome::Unavailable {
                reason: "technology-sharing member and bonus counts differ"
            }
        ));
        memory.put(GROUP + BONUS_COUNT, &2_u32.to_le_bytes());
        memory.put(0x6004, &0_u32.to_le_bytes());
        assert!(matches!(
            walk(&memory, BASE, GAME, &mut groups, &mut members).members,
            Outcome::Unavailable { .. }
        ));
        memory.put(GROUP + MEMBER_COUNT, &(-1_i32).to_le_bytes());
        assert!(matches!(
            walk(&memory, BASE, GAME, &mut groups, &mut members).groups,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn authenticated_empty_roster_does_not_require_unrelated_roots() {
        let mut memory = MemoryImage::default();
        memory.array(GAME + cgame_state::TECH_SHARING_GROUPS, 0, 0);
        let result = walk(&memory, BASE, GAME, &mut Vec::new(), &mut Vec::new());
        assert_eq!(result.groups, Outcome::Rows { len: 0 });
        assert_eq!(result.members, Outcome::Rows { len: 0 });
        memory.put(
            GAME + cgame_state::TECH_SHARING_GROUPS_COUNT,
            &1_u32.to_le_bytes(),
        );
        assert!(matches!(
            walk(&memory, BASE, GAME, &mut Vec::new(), &mut Vec::new()).groups,
            Outcome::Unavailable { .. }
        ));
    }
}
