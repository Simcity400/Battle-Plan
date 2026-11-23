
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use engine_native::{ccharacter, ccountry};
use crate::{cscientist, scientist_roster_entry, scientist_skill_entry};

pub const ENTRIES: usize = 0xa0;
pub const CAPACITY: usize = 0xa8;
pub const COUNT: usize = 0xac;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub ordinal: u32,
    pub character: u32,
    pub trait_count: i32,
    pub skill_count: i32,
    pub active: Option<u8>,
    pub injury_days: Option<i32>,
    pub template_visibility: Option<u8>,
    pub template_origin: Option<(usize, usize, u32)>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SkillRow {
    pub character: u32,
    pub ordinal: u32,
    pub token: u32,
    pub level: i32,
    pub experience_raw: i64,
}

pub struct Buffers<'a> {
    pub scientists: &'a mut Vec<Row>,
    pub skills: &'a mut Vec<SkillRow>,
    pub names: &'a mut Vec<u8>,
}

pub struct BuffersBuffer<'a> {
    pub scientists: engine_native_memory::walk::Buffer<'a, Row>,
    pub skills: engine_native_memory::walk::Buffer<'a, SkillRow>,
    pub names: engine_native_memory::walk::Buffer<'a, u8>,
}

impl<'a> From<Buffers<'a>> for BuffersBuffer<'a> {
    fn from(value: Buffers<'a>) -> Self {
        Self {
            scientists: engine_native_memory::walk::Buffer::Growing(value.scientists),
            skills: engine_native_memory::walk::Buffer::Growing(value.skills),
            names: engine_native_memory::walk::Buffer::Growing(value.names),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub scientists: Outcome,
    pub skills: Outcome,
}

#[derive(Default, Clone, Copy)]
struct Count {
    len: usize,
    unread: u32,
}

impl Count {
    fn push<T: Default>(
        &mut self,
        output: &mut engine_native_memory::walk::Buffer<'_, T>,
        row: T,
    ) -> Result<(), &'static str> {
        output.ensure(
            self.len
                .checked_add(1)
                .ok_or("scientist row count overflow")?,
        )?;
        if let Some(destination) = output.get_mut(self.len) {
            *destination = row;
            self.len += 1;
        } else {
            self.unread = self
                .unread
                .checked_add(1)
                .ok_or("scientist row count overflow")?;
        }
        Ok(())
    }

    fn outcome(self, failure: Option<&'static str>) -> Outcome {
        match failure {
            Some(reason) => Outcome::Unavailable { reason },
            None if self.unread == 0 => Outcome::Rows { len: self.len },
            None => Outcome::Truncated {
                len: self.len,
                unread_rows: self.unread,
            },
        }
    }
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Buffers<'_>,
) -> Walk {
    let mut buffers = BuffersBuffer {
        scientists: engine_native_memory::walk::Buffer::Growing(&mut *output.scientists),
        skills: engine_native_memory::walk::Buffer::Growing(&mut *output.skills),
        names: engine_native_memory::walk::Buffer::Growing(&mut *output.names),
    };
    walk_buffer(memory, base, game_state, &mut buffers)
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut BuffersBuffer<'_>,
) -> Walk {
    walk_buffer_with_injury(memory, base, game_state, output, true)
}

pub fn walk_with_injury(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Buffers<'_>,
    injury: bool,
) -> Walk {
    let mut buffers = BuffersBuffer {
        scientists: engine_native_memory::walk::Buffer::Growing(&mut *output.scientists),
        skills: engine_native_memory::walk::Buffer::Growing(&mut *output.skills),
        names: engine_native_memory::walk::Buffer::Growing(&mut *output.names),
    };
    walk_buffer_with_injury(memory, base, game_state, &mut buffers, injury)
}

fn walk_buffer_with_injury(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut BuffersBuffer<'_>,
    injury: bool,
) -> Walk {
    let mut counts = [Count::default(); 2];
    let mut names_used = 0;
    let failure = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let reader = Reader { memory, base };
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let roster = reader.optional(country.pointer, ccountry::SCIENTIST_ROSTER)?;
            if roster == 0 {
                continue;
            }
            let entries = roster
                .checked_add(ENTRIES)
                .and_then(|address| Array::read(memory, address, 8))
                .ok_or("scientist roster vector unreadable or invalid")?;
            for ordinal in 0..entries.count {
                let entry = entries
                    .element(ordinal, 8)
                    .and_then(|address| read::<u64>(memory, address))
                    .and_then(|value| usize::try_from(value).ok())
                    .filter(|&pointer| pointer != 0)
                    .ok_or("scientist roster entry unreadable or null")?;
                let character = reader.optional(entry, scientist_roster_entry::CHARACTER)?;
                if character == 0 {
                    return Err("scientist roster entry has a null character");
                }
                let id = reader.character(character)?;
                let scientist = reader.optional(character, ccharacter::SCIENTIST)?;
                if scientist == 0 {
                    return Err("rostered scientist character has no scientist role");
                }
                reader.vtable(
                    scientist,
                    cscientist::VTABLE_RVA,
                    "scientist vtable mismatch",
                )?;
                let trait_count = reader.field::<i32>(scientist, cscientist::TRAIT_COUNT)?;
                let skills = scientist
                    .checked_add(cscientist::SKILLS)
                    .and_then(|address| Array::read(memory, address, scientist_skill_entry::STRIDE))
                    .ok_or("scientist specialization vector unreadable or invalid")?;
                let skill_count =
                    i32::try_from(skills.count).map_err(|_| "scientist skill count overflow")?;
                if trait_count < 0 {
                    return Err("scientist trait count is negative");
                }
                let visibility = injury
                    .then(|| {
                        cscientist::visibility(
                            memory,
                            base,
                            scientist,
                            &mut output.names,
                            &mut names_used,
                        )
                    })
                    .and_then(Result::ok);
                counts[0].push(
                    &mut output.scientists,
                    Row {
                        country: country.id,
                        ordinal,
                        character: id,
                        trait_count,
                        skill_count,
                        active: reader.field(scientist, cscientist::ACTIVE).ok(),
                        template_visibility: visibility.map(|(state, _)| state),
                        template_origin: visibility.and_then(|(_, origin)| origin),
                        injury_days: injury
                            .then(|| reader.field(scientist, cscientist::INJURY_DAYS).ok())
                            .flatten(),
                    },
                )?;
                for index in 0..skills.count {
                    let record = skills
                        .element(index, scientist_skill_entry::STRIDE)
                        .ok_or("scientist specialization record out of range")?;
                    let token = reader.field::<u32>(record, scientist_skill_entry::TOKEN)?;
                    let level = reader.field::<i32>(record, scientist_skill_entry::LEVEL)?;
                    let experience_raw =
                        reader.field::<i64>(record, scientist_skill_entry::EXPERIENCE)?;
                    counts[1].push(
                        &mut output.skills,
                        SkillRow {
                            character: id,
                            ordinal: index,
                            token,
                            level,
                            experience_raw,
                        },
                    )?;
                }
            }
        }
        Ok(())
    })()
    .err();
    Walk {
        scientists: counts[0].outcome(failure),
        skills: counts[1].outcome(failure),
    }
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
            .ok_or("scientist field unreadable or address overflow")
    }

    fn optional(&self, pointer: usize, offset: usize) -> Result<usize, &'static str> {
        usize::try_from(self.field::<u64>(pointer, offset)?)
            .map_err(|_| "scientist pointer overflow")
    }

    fn vtable(
        &self,
        pointer: usize,
        rva: engine_sites::Rva,
        reason: &'static str,
    ) -> Result<(), &'static str> {
        let expected = engine_sites::at(self.base, rva).ok_or("module address overflow")? as u64;
        if read::<u64>(self.memory, pointer) == Some(expected) {
            Ok(())
        } else {
            Err(reason)
        }
    }

    fn character(&self, pointer: usize) -> Result<u32, &'static str> {
        let identity = read::<[u8; 16]>(self.memory, pointer)
            .ok_or("scientist character identity unreadable")?;
        let expected = engine_sites::at(self.base, ccharacter::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected {
            return Err("scientist character vtable mismatch");
        }
        let reference_type = u32::from_le_bytes(
            identity[ccharacter::TYPE..ccharacter::ID]
                .try_into()
                .unwrap(),
        );
        if reference_type != ccharacter::REFERENCE_TYPE {
            return Err("scientist character reference type mismatch");
        }
        let id = u32::from_le_bytes(identity[ccharacter::ID..16].try_into().unwrap());
        if id == 0 {
            return Err("scientist character has a zero id");
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use engine_native_memory::memory::SliceMemory;

    const GS: usize = 0x1000;
    const COUNTRIES: usize = 0x2000;
    const LINKS: usize = 0x2100;
    const C1: usize = 0x3000;
    const ROSTER: usize = 0x4000;
    const ENTRY_DATA: usize = 0x4200;
    const E1: usize = 0x4300;
    const E2: usize = 0x4400;
    const CH1: usize = 0x5000;
    const CH2: usize = 0x6000;
    const SC1: usize = 0x7000;
    const SC2: usize = 0x7400;
    const SKILLS1: usize = 0x8000;
    const SKILLS2: usize = 0x8200;

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn vector(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, count);
        p32(bytes, at + 12, count);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x9000];
        vector(&mut bytes, GS + cgame_state::COUNTRIES, COUNTRIES, 2);
        vector(&mut bytes, GS + cgame_state::COUNTRY_LINKS, LINKS, 2);
        p64(&mut bytes, COUNTRIES, u64::MAX);
        p64(&mut bytes, COUNTRIES + 8, C1 as u64);
        p32(&mut bytes, LINKS, 0);
        p32(&mut bytes, LINKS + 4, 1);
        p64(&mut bytes, C1, ccountry::VTABLE_RVA.0 as u64);
        p32(&mut bytes, C1 + ccountry::ID, 1);
        p64(&mut bytes, C1 + ccountry::SCIENTIST_ROSTER, ROSTER as u64);
        vector(&mut bytes, ROSTER + ENTRIES, ENTRY_DATA, 2);
        p64(&mut bytes, ENTRY_DATA, E1 as u64);
        p64(&mut bytes, ENTRY_DATA + 8, E2 as u64);
        p64(
            &mut bytes,
            E1 + scientist_roster_entry::CHARACTER,
            CH1 as u64,
        );
        p64(
            &mut bytes,
            E2 + scientist_roster_entry::CHARACTER,
            CH2 as u64,
        );
        for (character, id, scientist) in [(CH1, 265, SC1), (CH2, 266, SC2)] {
            p64(&mut bytes, character, ccharacter::VTABLE_RVA.0 as u64);
            p32(
                &mut bytes,
                character + ccharacter::TYPE,
                ccharacter::REFERENCE_TYPE,
            );
            p32(&mut bytes, character + ccharacter::ID, id);
            p64(
                &mut bytes,
                character + ccharacter::SCIENTIST,
                scientist as u64,
            );
            p64(&mut bytes, scientist, cscientist::VTABLE_RVA.0 as u64);
        }
        p64(&mut bytes, SC1 + cscientist::TRAIT_DATA, 0x8800);
        p32(&mut bytes, SC1 + cscientist::TRAIT_CAPACITY, 8);
        p32(&mut bytes, SC1 + cscientist::TRAIT_COUNT, 2);
        vector(&mut bytes, SC1 + cscientist::SKILLS, SKILLS1, 4);
        for index in 0..4_u32 {
            let record = SKILLS1 + index as usize * scientist_skill_entry::STRIDE;
            p32(
                &mut bytes,
                record + scientist_skill_entry::TOKEN,
                u32::from(scientist_skill_entry::NUCLEAR_TOKEN_PREFIX) + index,
            );
            p32(
                &mut bytes,
                record + scientist_skill_entry::LEVEL,
                if index == 0 { 2 } else { 0 },
            );
            p64(
                &mut bytes,
                record + scientist_skill_entry::EXPERIENCE,
                if index == 1 { 21_918_000 } else { 0 },
            );
        }
        vector(&mut bytes, SC2 + cscientist::SKILLS, SKILLS2, 0);
        bytes[SC1 + cscientist::ACTIVE] = 253;
        bytes
    }

    fn run(bytes: &[u8], capacity: [usize; 2]) -> (Walk, Vec<Row>, Vec<SkillRow>) {
        let mut scientists = vec![Row::default(); capacity[0]];
        let mut skills = vec![SkillRow::default(); capacity[1]];
        let result = walk(
            &SliceMemory { base: 0, bytes },
            0,
            GS,
            &mut Buffers {
                scientists: &mut scientists,
                skills: &mut skills,
                names: &mut Vec::new(),
            },
        );
        (result, scientists, skills)
    }

    #[test]
    fn gap_research_grants_scientist_injury_is_demanded_only_by_the_daily_supply_read() {
        struct NoInjury<'a>(SliceMemory<'a>, std::cell::Cell<usize>);
        impl Memory for NoInjury<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if [
                    SC1 + cscientist::INJURY_DAYS,
                    SC2 + cscientist::INJURY_DAYS,
                    SC1 + cscientist::VISIBILITY_TEMPLATE,
                    SC2 + cscientist::VISIBILITY_TEMPLATE,
                ]
                .contains(&address)
                {
                    self.1.set(self.1.get() + 1);
                }
                self.0.copy(address, destination)
            }
        }
        let bytes = fixture();
        let mut scientists = Vec::new();
        let mut skills = Vec::new();
        let mut names = Vec::new();
        let mut output = Buffers {
            scientists: &mut scientists,
            skills: &mut skills,
            names: &mut names,
        };
        let memory = NoInjury(
            SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            std::cell::Cell::new(0),
        );
        let walk = walk_with_injury(&memory, 0, GS, &mut output, false);
        assert!(matches!(walk.scientists, Outcome::Rows { len: 2 }));
        assert!(
            output.scientists[..2]
                .iter()
                .all(|row| row.injury_days.is_none())
        );
        assert_eq!(memory.1.get(), 0);
        let walk = walk_with_injury(&memory, 0, GS, &mut output, true);
        assert!(matches!(walk.scientists, Outcome::Rows { len: 2 }));
        assert!(
            output.scientists[..2]
                .iter()
                .all(|row| row.injury_days == Some(0))
        );
        assert_eq!(memory.1.get(), 4);
    }

    #[test]
    fn roster_order_native_counts_and_raw_levels_survive_without_clamping() {
        let bytes = fixture();
        let (result, scientists, skills) = run(&bytes, [4, 8]);
        assert_eq!(result.scientists, Outcome::Rows { len: 2 });
        assert_eq!(result.skills, Outcome::Rows { len: 4 });
        assert_eq!(
            &scientists[..2],
            &[
                Row {
                    country: 1,
                    ordinal: 0,
                    character: 265,
                    trait_count: 2,
                    skill_count: 4,
                    active: Some(253),
                    injury_days: Some(0),
                    template_visibility: Some(0),
                    template_origin: None,
                },
                Row {
                    country: 1,
                    ordinal: 1,
                    character: 266,
                    trait_count: 0,
                    skill_count: 0,
                    active: Some(0),
                    injury_days: Some(0),
                    template_visibility: Some(0),
                    template_origin: None,
                },
            ]
        );
        assert_eq!(
            skills[0],
            SkillRow {
                character: 265,
                ordinal: 0,
                token: u32::from(scientist_skill_entry::NUCLEAR_TOKEN_PREFIX),
                level: 2,
                experience_raw: 0,
            }
        );
        assert_eq!(skills[1].experience_raw, 21_918_000);
        assert_eq!(skills[1].level, 0);
        assert_eq!(skills[3].level, 0);
        assert_eq!(
            skills[3].token,
            u32::from(scientist_skill_entry::LAND_TOKEN_PREFIX)
        );
    }

    #[test]
    fn unreadable_activity_preserves_roster_and_specializations() {
        struct ActivityFault<'a>(&'a [u8]);
        impl Memory for ActivityFault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                address != SC1 + cscientist::ACTIVE
                    && SliceMemory {
                        base: 0,
                        bytes: self.0,
                    }
                    .copy(address, output)
            }
        }
        let bytes = fixture();
        let (_, expected, expected_skills) = run(&bytes, [2, 4]);
        let mut scientists = vec![Row::default(); 2];
        let mut skills = vec![SkillRow::default(); 4];
        let result = walk(
            &ActivityFault(&bytes),
            0,
            GS,
            &mut Buffers {
                scientists: &mut scientists,
                skills: &mut skills,
                names: &mut Vec::new(),
            },
        );
        assert_eq!(result.scientists, Outcome::Rows { len: 2 });
        assert_eq!(result.skills, Outcome::Rows { len: 4 });
        assert_eq!(
            scientists[0],
            Row {
                active: None,
                ..expected[0]
            }
        );
        assert_eq!(scientists[1], expected[1]);
        assert_eq!(skills.as_slice(), expected_skills);
    }

    #[test]
    fn native_signed_levels_and_wide_tokens_are_preserved_exactly() {
        let mut bytes = fixture();
        p32(
            &mut bytes,
            SKILLS1 + scientist_skill_entry::LEVEL,
            -7_i32 as u32,
        );
        p32(
            &mut bytes,
            SKILLS1 + scientist_skill_entry::TOKEN,
            0xdead_56c6,
        );
        p64(
            &mut bytes,
            SKILLS1 + scientist_skill_entry::EXPERIENCE,
            i64::MIN as u64,
        );
        let (_, _, skills) = run(&bytes, [4, 8]);
        assert_eq!(skills[0].level, -7);
        assert_eq!(skills[0].token, 0xdead_56c6);
        assert_eq!(skills[0].experience_raw, i64::MIN);
    }

    #[test]
    fn a_missing_roster_object_is_absence_while_a_broken_one_fails() {
        let mut bytes = fixture();
        p64(&mut bytes, C1 + ccountry::SCIENTIST_ROSTER, 0);
        let (result, ..) = run(&bytes, [4, 8]);
        assert_eq!(result.scientists, Outcome::Rows { len: 0 });
        assert_eq!(result.skills, Outcome::Rows { len: 0 });

        for (offset, value) in [
            (ENTRY_DATA, 0_u64),
            (E1 + scientist_roster_entry::CHARACTER, 0),
            (CH1, 1),
            (SC1, 1),
        ] {
            let mut bytes = fixture();
            p64(&mut bytes, offset, value);
            let (result, ..) = run(&bytes, [4, 8]);
            assert!(
                matches!(result.scientists, Outcome::Unavailable { .. }),
                "{offset:#x}"
            );
            assert!(matches!(result.skills, Outcome::Unavailable { .. }));
        }

        let mut bytes = fixture();
        p64(&mut bytes, CH2 + ccharacter::SCIENTIST, 0);
        let (result, ..) = run(&bytes, [4, 8]);
        assert_eq!(
            result.scientists,
            Outcome::Unavailable {
                reason: "rostered scientist character has no scientist role"
            }
        );

        let mut bytes = fixture();
        p32(&mut bytes, SC1 + cscientist::TRAIT_COUNT, u32::MAX);
        let (result, ..) = run(&bytes, [4, 8]);
        assert_eq!(
            result.scientists,
            Outcome::Unavailable {
                reason: "scientist trait count is negative"
            }
        );
    }

    #[test]
    fn an_unreadable_world_yields_no_prefix() {
        struct Unreadable;
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        let mut scientists = vec![Row::default(); 2];
        let mut skills = vec![SkillRow::default(); 2];
        let result = walk(
            &Unreadable,
            0,
            GS,
            &mut Buffers {
                scientists: &mut scientists,
                skills: &mut skills,
                names: &mut Vec::new(),
            },
        );
        assert!(matches!(result.scientists, Outcome::Unavailable { .. }));
        assert!(matches!(result.skills, Outcome::Unavailable { .. }));
    }
}
