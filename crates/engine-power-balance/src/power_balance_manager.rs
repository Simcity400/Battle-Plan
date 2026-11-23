
pub const RECORDS: usize = 0x08;
#[cfg(test)]
pub const RECORD_COUNT: usize = 0x14;

use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    msvc_string,
    read,
    read_slice,
    walk::Outcome,
};
use engine_native::{ccountry, cgame_state};
use engine_native_parts::{modifier_accumulator, modifier_definition};
use crate::{
    modifier_keys,
    power_balance_definition,
    power_balance_gfx_override,
    power_balance_range,
    power_balance_record,
    power_balance_side,
};

use power_balance_definition::Token;

pub struct Buffers<'a> {
    pub records: engine_native_memory::walk::Buffer<'a, power_balance_record::Row>,
    pub members: engine_native_memory::walk::Buffer<'a, power_balance_record::MemberRow>,
    pub ranges: engine_native_memory::walk::Buffer<'a, power_balance_range::Row>,
    pub overrides: engine_native_memory::walk::Buffer<'a, power_balance_gfx_override::Row>,
    pub text: engine_native_memory::walk::Buffer<'a, u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Census {
    pub records: Outcome,
    pub members: Outcome,
    pub ranges: Outcome,
    pub overrides: Outcome,
    pub text_len: usize,
    pub required_text: usize,
}

#[derive(Default)]
struct Count {
    len: usize,
    unread: u32,
}

impl Count {
    fn omit(&mut self, count: u32) -> Result<(), &'static str> {
        self.unread = self
            .unread
            .checked_add(count)
            .ok_or("power balance row count overflow")?;
        Ok(())
    }

    fn outcome(&self) -> Outcome {
        if self.unread == 0 {
            Outcome::Rows { len: self.len }
        } else {
            Outcome::Truncated {
                len: self.len,
                unread_rows: self.unread,
            }
        }
    }
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffers<'_>,
) -> Census {
    let result = (|| {
        let manager = game_state
            .checked_add(cgame_state::POWER_BALANCE_MANAGER)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("power balance manager unreadable or null")?;
        let roster = manager
            .checked_add(RECORDS)
            .and_then(|p| Array::read(memory, p, power_balance_record::STRIDE))
            .ok_or("power balance record array unreadable or invalid")?;
        output.records.ensure(roster.count as usize)?;
        let definitions = engine_sites::at(base, modifier_definition::TABLE_RVA)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("power balance modifier definitions unreadable or null")?;
        let daily_flags = flags(memory, definitions, modifier_keys::POWER_BALANCE_DAILY)?;
        let weekly_flags = flags(memory, definitions, modifier_keys::POWER_BALANCE_WEEKLY)?;
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut records = Count::default();
        let mut members = Count::default();
        let mut ranges = Count::default();
        let mut overrides = Count::default();
        let mut text_len = 0;
        let mut required_text = 0;
        for ordinal in 0..roster.count {
            let address = roster
                .element(ordinal, power_balance_record::STRIDE)
                .ok_or("power balance record address overflow")?;
            let fixed = read::<[u8; power_balance_record::STRIDE]>(memory, address)
                .ok_or("power balance record unreadable")?;
            if fixed[power_balance_record::VALID] == 0 {
                return Err("power balance record validity byte is zero");
            }
            let sources = [
                identity(
                    memory,
                    &fixed,
                    power_balance_record::DEFINITION,
                    power_balance_definition::IDENTITY,
                )?,
                identity(
                    memory,
                    &fixed,
                    power_balance_record::LEFT_SIDE,
                    power_balance_side::IDENTITY,
                )?,
                identity(
                    memory,
                    &fixed,
                    power_balance_record::RIGHT_SIDE,
                    power_balance_side::IDENTITY,
                )?,
                identity(
                    memory,
                    &fixed,
                    power_balance_record::TRENDING_SIDE,
                    power_balance_side::IDENTITY,
                )?,
                identity(
                    memory,
                    &fixed,
                    power_balance_record::ACTIVE_RANGE,
                    power_balance_range::IDENTITY,
                )?,
            ];
            let tokens = copy_tokens(
                memory,
                sources,
                records.len < output.records.len(),
                &mut output.text,
                &mut text_len,
                &mut required_text,
            )?;
            if let Some([balance, left, right, trending, active_range]) = tokens {
                output.records[records.len] = power_balance_record::Row {
                    ordinal,
                    value_raw: i64::from_le_bytes(
                        fixed[power_balance_record::VALUE..power_balance_record::VALUE + 8]
                            .try_into()
                            .unwrap(),
                    ),
                    balance,
                    left,
                    right,
                    trending,
                    active_range,
                };
                records.len += 1;
            } else {
                records.omit(1)?;
            }

            let member_array = array(
                &fixed,
                power_balance_record::MEMBERS,
                power_balance_record::MEMBER_COUNT,
                power_balance_record::MEMBER_STRIDE,
            )?;
            output.members.ensure(
                members
                    .len
                    .checked_add(member_array.count as usize)
                    .ok_or("power balance members demand overflow")?,
            )?;
            for member_ordinal in 0..member_array.count {
                if members.len == output.members.len() {
                    members.omit(member_array.count - member_ordinal)?;
                    break;
                }
                let raw = member_array
                    .element(member_ordinal, power_balance_record::MEMBER_STRIDE)
                    .and_then(|p| read::<u32>(memory, p))
                    .ok_or("power balance member reference unreadable")?;
                let country = countries.resolve(raw)?;
                let mut values = [engine_native_memory::modifier_cache::Value {
                    raw: 0,
                    present: false,
                }; 2];
                modifier_accumulator::lookup_many(
                    memory,
                    country.pointer,
                    &[
                        modifier_keys::POWER_BALANCE_DAILY,
                        modifier_keys::POWER_BALANCE_WEEKLY,
                    ],
                    &mut values,
                )?;
                output.members[members.len] = power_balance_record::MemberRow {
                    record: ordinal,
                    ordinal: member_ordinal,
                    country: raw,
                    daily_raw: values[0].raw,
                    weekly_raw: values[1].raw,
                    daily_present: values[0].present,
                    weekly_present: values[1].present,
                    daily_flags,
                    weekly_flags,
                };
                members.len += 1;
            }

            let range_array = array(
                &fixed,
                power_balance_record::RANGES,
                power_balance_record::RANGE_COUNT,
                power_balance_record::RANGE_STRIDE,
            )?;
            output.ranges.ensure(
                ranges
                    .len
                    .checked_add(range_array.count as usize)
                    .ok_or("power balance ranges demand overflow")?,
            )?;
            for range_ordinal in 0..range_array.count {
                let pointer = range_array
                    .element(range_ordinal, power_balance_record::RANGE_STRIDE)
                    .and_then(|p| read::<u64>(memory, p))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .ok_or("power balance range pointer unreadable or null")?;
                let source = string(
                    memory,
                    pointer
                        .checked_add(power_balance_range::IDENTITY)
                        .ok_or("power balance range identity address overflow")?,
                )?;
                if let Some([identity]) = copy_tokens(
                    memory,
                    [source],
                    ranges.len < output.ranges.len(),
                    &mut output.text,
                    &mut text_len,
                    &mut required_text,
                )? {
                    output.ranges[ranges.len] = power_balance_range::Row {
                        record: ordinal,
                        ordinal: range_ordinal,
                        identity,
                    };
                    ranges.len += 1;
                } else {
                    ranges.omit(1)?;
                }
            }

            let override_array = array(
                &fixed,
                power_balance_record::GFX_OVERRIDES,
                power_balance_record::GFX_OVERRIDE_COUNT,
                power_balance_gfx_override::STRIDE,
            )?;
            let capacity = i32_at(&fixed, power_balance_record::GFX_OVERRIDE_CAPACITY)?;
            if capacity < 0 || override_array.count > capacity as u32 {
                return Err("power balance override capacity invalid");
            }
            output.overrides.ensure(
                overrides
                    .len
                    .checked_add(override_array.count as usize)
                    .ok_or("power balance overrides demand overflow")?,
            )?;
            for override_ordinal in 0..override_array.count {
                let pointer = override_array
                    .element(override_ordinal, power_balance_gfx_override::STRIDE)
                    .ok_or("power balance override address overflow")?;
                let side = string(
                    memory,
                    pointer
                        .checked_add(power_balance_gfx_override::SIDE)
                        .ok_or("power balance override side address overflow")?,
                )?;
                let gfx = string(
                    memory,
                    pointer
                        .checked_add(power_balance_gfx_override::GFX)
                        .ok_or("power balance override gfx address overflow")?,
                )?;
                if let Some([side, gfx]) = copy_tokens(
                    memory,
                    [side, gfx],
                    overrides.len < output.overrides.len(),
                    &mut output.text,
                    &mut text_len,
                    &mut required_text,
                )? {
                    output.overrides[overrides.len] = power_balance_gfx_override::Row {
                        record: ordinal,
                        ordinal: override_ordinal,
                        side,
                        gfx,
                    };
                    overrides.len += 1;
                } else {
                    overrides.omit(1)?;
                }
            }
        }
        Ok(Census {
            records: records.outcome(),
            members: members.outcome(),
            ranges: ranges.outcome(),
            overrides: overrides.outcome(),
            text_len,
            required_text,
        })
    })();
    result.unwrap_or_else(|reason| Census {
        records: Outcome::Unavailable { reason },
        members: Outcome::Unavailable { reason },
        ranges: Outcome::Unavailable { reason },
        overrides: Outcome::Unavailable { reason },
        text_len: 0,
        required_text: 0,
    })
}

fn flags(memory: &impl Memory, definitions: usize, key: u32) -> Result<u32, &'static str> {
    let pointer = (key as usize)
        .checked_mul(modifier_definition::STRIDE)
        .and_then(|p| definitions.checked_add(p))
        .ok_or("power balance modifier definition address overflow")?;
    let feature = pointer
        .checked_add(modifier_definition::FEATURE_SELECTOR)
        .and_then(|p| read::<u32>(memory, p))
        .ok_or("power balance modifier feature unreadable")?;
    if feature != 0 {
        return Err("power balance modifier feature requires an engine callback");
    }
    pointer
        .checked_add(modifier_definition::FLAGS)
        .and_then(|p| read::<u32>(memory, p))
        .ok_or("power balance modifier flags unreadable")
}

fn i32_at(fixed: &[u8], offset: usize) -> Result<i32, &'static str> {
    let bytes = fixed
        .get(
            offset
                ..offset
                    .checked_add(4)
                    .ok_or("power balance field address overflow")?,
        )
        .ok_or("power balance field outside record")?;
    Ok(i32::from_le_bytes(bytes.try_into().unwrap()))
}

fn pointer_at(fixed: &[u8], offset: usize) -> Result<usize, &'static str> {
    let bytes = fixed
        .get(
            offset
                ..offset
                    .checked_add(8)
                    .ok_or("power balance field address overflow")?,
        )
        .ok_or("power balance field outside record")?;
    usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
        .map_err(|_| "power balance pointer overflow")
}

fn array(fixed: &[u8], data: usize, count: usize, stride: usize) -> Result<Array, &'static str> {
    let data = pointer_at(fixed, data)?;
    let count = i32_at(fixed, count)?;
    if count < 0 || (count != 0 && data == 0) {
        return Err("power balance child array invalid");
    }
    data.checked_add(
        (count as usize)
            .checked_mul(stride)
            .ok_or("power balance child array size overflow")?,
    )
    .ok_or("power balance child array address overflow")?;
    Ok(Array {
        data,
        count: count as u32,
    })
}

#[derive(Clone, Copy)]
struct Source {
    address: usize,
    len: usize,
}

fn identity(
    memory: &impl Memory,
    fixed: &[u8],
    field: usize,
    offset: usize,
) -> Result<Source, &'static str> {
    let pointer = pointer_at(fixed, field)?;
    if pointer == 0 {
        return Err("power balance definition pointer is null");
    }
    string(
        memory,
        pointer
            .checked_add(offset)
            .ok_or("power balance identity address overflow")?,
    )
}

fn string(memory: &impl Memory, address: usize) -> Result<Source, &'static str> {
    let header = read::<[u8; msvc_string::STRIDE]>(memory, address)
        .ok_or("power balance string header unreadable")?;
    let len = pointer_at(&header, msvc_string::LENGTH)?;
    let capacity = pointer_at(&header, msvc_string::CAPACITY)?;
    if capacity < msvc_string::INLINE_CAPACITY as usize || len > capacity {
        return Err("power balance string header invalid");
    }
    let source = if capacity == msvc_string::INLINE_CAPACITY as usize {
        address
    } else {
        pointer_at(&header, 0)?
    };
    if source == 0 {
        return Err("power balance string source is null");
    }
    if source.checked_add(len).and_then(|p| read::<u8>(memory, p)) != Some(0) {
        return Err("power balance string terminator unreadable or invalid");
    }
    Ok(Source {
        address: source,
        len,
    })
}

fn copy_tokens<const N: usize>(
    memory: &impl Memory,
    sources: [Source; N],
    row_fits: bool,
    arena: &mut engine_native_memory::walk::Buffer<'_, u8>,
    used: &mut usize,
    required: &mut usize,
) -> Result<Option<[Token; N]>, &'static str> {
    let mut total = 0usize;
    for source in sources {
        total = total
            .checked_add(source.len)
            .ok_or("power balance token bytes overflow")?;
    }
    *required = required
        .checked_add(total)
        .ok_or("power balance arena size overflow")?;
    arena.ensure(*required)?;
    if !row_fits || total > arena.len() - *used {
        return Ok(None);
    }
    let mut tokens = [Token::default(); N];
    for (token, source) in tokens.iter_mut().zip(sources) {
        *token = Token {
            start: *used,
            len: source.len,
        };
        if !read_slice(
            memory,
            source.address,
            &mut arena[*used..*used + source.len],
        ) {
            return Err("power balance string bytes unreadable");
        }
        *used += source.len;
    }
    Ok(Some(tokens))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const GS: usize = 0x1000;
    const MANAGER: usize = 0x2000;
    const RECORD: usize = 0x3000;
    const COUNTRY: usize = 0x4000;
    const MEMBER_DATA: usize = 0x5000;
    const MODIFIER_DATA: usize = 0x5100;
    const RANGE_DATA: usize = 0x5200;
    const OVERRIDE_DATA: usize = 0x5300;
    const DEFINITIONS: usize = 0x10000;
    const STRINGS: usize = 0x20000;
    const RANGE: usize = 0x21000;

    struct MemoryMap(BTreeMap<usize, Vec<u8>>);

    impl Memory for MemoryMap {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let Some((&base, region)) = self.0.range(..=address).next_back() else {
                return false;
            };
            let start = address - base;
            let Some(end) = start.checked_add(destination.len()) else {
                return false;
            };
            let Some(source) = region.get(start..end) else {
                return false;
            };
            destination.copy_from_slice(source);
            true
        }
    }

    impl MemoryMap {
        fn write(&mut self, address: usize, bytes: &[u8]) {
            let (&base, region) = self.0.range_mut(..=address).next_back().unwrap();
            region[address - base..address - base + bytes.len()].copy_from_slice(bytes);
        }

        fn qword(&mut self, address: usize, value: u64) {
            self.write(address, &value.to_le_bytes());
        }
        fn dword(&mut self, address: usize, value: u32) {
            self.write(address, &value.to_le_bytes());
        }

        fn header(&mut self, address: usize, data: usize, count: u32) {
            self.qword(address, data as u64);
            self.dword(address + 8, count);
            self.dword(address + 12, count);
        }

        fn text(&mut self, address: usize, bytes: &[u8]) {
            assert!(bytes.len() <= 15);
            self.write(address, bytes);
            self.write(address + bytes.len(), &[0]);
            self.qword(address + 16, bytes.len() as u64);
            self.qword(address + 24, 15);
        }
    }

    fn fixture() -> MemoryMap {
        let mut m = MemoryMap(BTreeMap::from([
            (GS, vec![0; 0x1000]),
            (MANAGER, vec![0; 0x100]),
            (RECORD, vec![0; power_balance_record::STRIDE]),
            (COUNTRY, vec![0; 0x1000]),
            (MEMBER_DATA, vec![0; 0x1000]),
            (DEFINITIONS, vec![0; 0x5000]),
            (STRINGS, vec![0; 0x1000]),
            (RANGE, vec![0; 0x1000]),
            (modifier_definition::TABLE_RVA.0, vec![0; 8]),
        ]));
        m.qword(GS + cgame_state::POWER_BALANCE_MANAGER, MANAGER as u64);
        m.header(MANAGER + RECORDS, RECORD, 1);
        m.qword(modifier_definition::TABLE_RVA.0, DEFINITIONS as u64);
        m.dword(
            DEFINITIONS
                + modifier_keys::POWER_BALANCE_DAILY as usize * modifier_definition::STRIDE
                + modifier_definition::FLAGS,
            modifier_definition::CLAMP_MAXIMUM,
        );
        m.header(GS + cgame_state::COUNTRIES, GS + 0x800, 2);
        m.qword(GS + 0x808, COUNTRY as u64);
        m.header(GS + cgame_state::COUNTRY_LINKS, GS + 0x900, 3);
        m.dword(GS + 0x904, 1);
        m.dword(GS + 0x908, 1);
        m.qword(COUNTRY, ccountry::VTABLE_RVA.0 as u64);
        m.dword(COUNTRY + ccountry::ID, 1);
        m.header(COUNTRY + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, MODIFIER_DATA, 2);
        m.dword(MODIFIER_DATA, modifier_keys::POWER_BALANCE_DAILY);
        m.qword(MODIFIER_DATA + 8, i64::MIN as u64);
        m.dword(MODIFIER_DATA + 16, modifier_keys::POWER_BALANCE_WEEKLY);
        m.qword(MODIFIER_DATA + 24, i64::MAX as u64);
        m.write(RECORD + power_balance_record::VALID, &[1]);
        m.qword(RECORD + power_balance_record::VALUE, (-12_345_i64) as u64);
        for (index, (field, token)) in [
            (power_balance_record::DEFINITION, b"balance".as_slice()),
            (power_balance_record::LEFT_SIDE, b"left".as_slice()),
            (power_balance_record::RIGHT_SIDE, b"right".as_slice()),
            (power_balance_record::TRENDING_SIDE, b"left".as_slice()),
        ]
        .into_iter()
        .enumerate()
        {
            let definition = STRINGS + index * 0x100;
            m.qword(RECORD + field, definition as u64);
            m.text(definition + 0x10, token);
        }
        m.qword(RECORD + power_balance_record::ACTIVE_RANGE, RANGE as u64);
        m.text(RANGE + power_balance_range::IDENTITY, b"range");
        m.header(RECORD + power_balance_record::MEMBERS, MEMBER_DATA, 1);
        m.dword(MEMBER_DATA, 2);
        m.header(RECORD + power_balance_record::RANGES, RANGE_DATA, 1);
        m.qword(RANGE_DATA, RANGE as u64);
        m.header(
            RECORD + power_balance_record::GFX_OVERRIDES,
            OVERRIDE_DATA,
            2,
        );
        m.text(OVERRIDE_DATA + power_balance_gfx_override::SIDE, b"left");
        m.text(OVERRIDE_DATA + power_balance_gfx_override::GFX, b"");
        m.text(
            OVERRIDE_DATA + power_balance_gfx_override::STRIDE + power_balance_gfx_override::SIDE,
            b"left",
        );
        m.text(
            OVERRIDE_DATA + power_balance_gfx_override::STRIDE + power_balance_gfx_override::GFX,
            &[0xff, 0, b'X'],
        );
        m
    }

    #[derive(Default)]
    struct Output {
        records: Vec<power_balance_record::Row>,
        members: Vec<power_balance_record::MemberRow>,
        ranges: Vec<power_balance_range::Row>,
        overrides: Vec<power_balance_gfx_override::Row>,
        text: Vec<u8>,
    }

    impl Output {
        fn buffers(&mut self) -> Buffers<'_> {
            use engine_native_memory::walk::Buffer;
            Buffers {
                records: Buffer::Growing(&mut self.records),
                members: Buffer::Growing(&mut self.members),
                ranges: Buffer::Growing(&mut self.ranges),
                overrides: Buffer::Growing(&mut self.overrides),
                text: Buffer::Growing(&mut self.text),
            }
        }
    }

    #[test]
    fn worker_grows_every_power_balance_row_and_arena_from_original_mod_sized_rosters() {
        use engine_native_memory::walk::Buffer;
        let mut memory = fixture();
        let mut records = Vec::new();
        let mut members = Vec::new();
        let mut ranges = Vec::new();
        let mut overrides = Vec::new();
        let mut text = Vec::new();
        let first = walk(
            &memory,
            0,
            GS,
            Buffers {
                records: Buffer::Growing(&mut records),
                members: Buffer::Growing(&mut members),
                ranges: Buffer::Growing(&mut ranges),
                overrides: Buffer::Growing(&mut overrides),
                text: Buffer::Growing(&mut text),
            },
        );
        assert_eq!(first.records, Outcome::Rows { len: 1 });
        assert_eq!(first.members, Outcome::Rows { len: 1 });
        assert_eq!(first.ranges, Outcome::Rows { len: 1 });
        assert_eq!(first.overrides, Outcome::Rows { len: 2 });
        assert_eq!(first.text_len, first.required_text);
        let template = memory.0[&RECORD].clone();
        let data = 0x30000;
        memory.0.insert(
            data,
            (0..5_000).flat_map(|_| template.iter().copied()).collect(),
        );
        memory.header(MANAGER + RECORDS, data, 5_000);
        let later = walk(
            &memory,
            0,
            GS,
            Buffers {
                records: Buffer::Growing(&mut records),
                members: Buffer::Growing(&mut members),
                ranges: Buffer::Growing(&mut ranges),
                overrides: Buffer::Growing(&mut overrides),
                text: Buffer::Growing(&mut text),
            },
        );
        assert_eq!(later.records, Outcome::Rows { len: 5_000 });
        assert_eq!(later.members, Outcome::Rows { len: 5_000 });
        assert_eq!(later.ranges, Outcome::Rows { len: 5_000 });
        assert_eq!(later.overrides, Outcome::Rows { len: 10_000 });
        assert_eq!(later.required_text, first.required_text * 5_000);
        assert_eq!(later.text_len, later.required_text);
        for (index, record) in records[..5_000].iter().enumerate() {
            assert_eq!(record.ordinal as usize, index);
            assert_eq!(record.balance.bytes(&text), Some(b"balance".as_slice()));
            assert_eq!(members[index].record as usize, index);
            assert_eq!(
                (members[index].daily_raw, members[index].weekly_raw),
                (i64::MIN, i64::MAX)
            );
            assert_eq!(
                ranges[index].identity.bytes(&text),
                Some(b"range".as_slice())
            );
            assert_eq!(
                overrides[index * 2 + 1].gfx.bytes(&text),
                Some([0xff, 0, b'X'].as_slice())
            );
        }
    }

    #[test]
    fn census_preserves_native_values_aliases_ranges_and_raw_override_order() {
        let m = fixture();
        let mut output = Output::default();
        let census = walk(&m, 0, GS, output.buffers());
        assert_eq!(census.records, Outcome::Rows { len: 1 });
        assert_eq!(census.members, Outcome::Rows { len: 1 });
        assert_eq!(census.ranges, Outcome::Rows { len: 1 });
        assert_eq!(census.overrides, Outcome::Rows { len: 2 });
        assert_eq!(census.text_len, census.required_text);
        assert_eq!(output.records[0].value_raw, -12_345);
        assert_eq!(
            output.records[0].balance.bytes(&output.text),
            Some(b"balance".as_slice())
        );
        assert_eq!(output.members[0].country, 2);
        assert_eq!(output.members[0].daily_raw, i64::MIN);
        assert_eq!(output.members[0].daily_effective(), 0);
        assert_eq!(output.members[0].weekly_raw, i64::MAX);
        assert_eq!(output.members[0].weekly_effective(), i64::MAX);
        assert!(output.members[0].daily_present && output.members[0].weekly_present);
        assert_eq!(
            output.ranges[0].identity.bytes(&output.text),
            Some(b"range".as_slice())
        );
        assert_eq!(
            output.overrides[0].gfx.bytes(&output.text),
            Some(b"".as_slice())
        );
        assert_eq!(output.overrides[1].ordinal, 1);
        assert_eq!(
            output.overrides[1].gfx.bytes(&output.text),
            Some([0xff, 0, b'X'].as_slice())
        );
    }

    #[test]
    fn missing_manager_invalid_members_and_feature_callbacks_never_claim_empty_success() {
        let mut output = Output::default();
        let mut missing = fixture();
        missing.qword(GS + cgame_state::POWER_BALANCE_MANAGER, 0);
        let result = walk(&missing, 0, GS, output.buffers());
        assert!(matches!(result.records, Outcome::Unavailable { .. }));
        assert_eq!(result.records, result.members);
        let mut broken_member = fixture();
        broken_member.dword(MEMBER_DATA, 0);
        assert!(matches!(
            walk(&broken_member, 0, GS, output.buffers()).members,
            Outcome::Unavailable { .. }
        ));
        let mut callback = fixture();
        callback.dword(
            DEFINITIONS
                + modifier_keys::POWER_BALANCE_DAILY as usize * modifier_definition::STRIDE
                + modifier_definition::FEATURE_SELECTOR,
            1,
        );
        assert_eq!(
            walk(&callback, 0, GS, output.buffers()).records,
            Outcome::Unavailable {
                reason: "power balance modifier feature requires an engine callback"
            }
        );
    }

    #[test]
    fn complete_empty_manager_is_distinct_from_bad_array_or_invalid_record() {
        let mut output = Output::default();
        let mut empty = fixture();
        empty.header(MANAGER + RECORDS, 0, 0);
        let result = walk(&empty, 0, GS, output.buffers());
        assert_eq!(result.records, Outcome::Rows { len: 0 });
        assert_eq!(result.members, Outcome::Rows { len: 0 });
        assert_eq!(result.ranges, Outcome::Rows { len: 0 });
        assert_eq!(result.overrides, Outcome::Rows { len: 0 });
        empty.dword(MANAGER + RECORD_COUNT, 1);
        assert!(matches!(
            walk(&empty, 0, GS, output.buffers()).records,
            Outcome::Unavailable { .. }
        ));
        let mut invalid = fixture();
        invalid.write(RECORD + power_balance_record::VALID, &[0]);
        assert!(matches!(
            walk(&invalid, 0, GS, output.buffers()).records,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn long_heap_identity_is_exact_and_unreadable_bytes_refuse_the_census() {
        let mut m = fixture();
        let token = [b'q'; 200];
        let pointer = STRINGS + 0x800;
        m.write(pointer, &token);
        m.write(pointer + token.len(), &[0]);
        m.qword(STRINGS + 0x10, pointer as u64);
        m.qword(STRINGS + 0x20, token.len() as u64);
        m.qword(STRINGS + 0x28, 255);
        let mut output = Output::default();
        assert_eq!(
            walk(&m, 0, GS, output.buffers()).records,
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            output.records[0].balance.bytes(&output.text),
            Some(token.as_slice())
        );
        m.write(pointer + token.len(), &[1]);
        assert!(matches!(
            walk(&m, 0, GS, output.buffers()).records,
            Outcome::Unavailable { .. }
        ));
    }
}
