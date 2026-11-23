
use engine_sites::Rva;
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    read,
    read_slice,
    walk::{Buffer, Outcome},
};
use engine_native::ccountry;
use crate::{
    cdecision,
    cdecision_cooldown,
    ctargeted_decision::{self, TargetKind},
    ctimed_decision,
};

pub const VTABLE_RVA: Rva = Rva(0x0286_8760);
pub const COUNTRY: usize = 0x08;
pub const DECISIONS_TAKEN: usize = 0x40;
pub const RE_ENABLE: usize = 0x58;
pub const REMOVE: usize = 0x70;
pub const ACTIVE_TIMED: usize = 0xa0;
pub const TARGETED: usize = 0xe8;
pub const TARGETED_TIMED: usize = 0x100;
pub const RANDOM_ITEMS: usize = 0x148;
pub const RANDOM_ITEM_STRIDE: usize = 0x18;
pub const RANDOM_ITEM_VTABLE_RVA: Rva = Rva(0x0286_8710);
pub const RANDOM_ITEM_DEFINITION: usize = 0x08;
pub const RANDOM_ITEM_SEED: usize = 0x10;
pub const RANDOM_ITEM_TARGET: usize = 0x14;
pub const RECORD_POINTER_STRIDE: usize = 8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Container {
    #[default]
    ReEnable = 0,
    Remove = 1,
    Timed = 2,
    Targeted = 3,
    TargetedTimed = 4,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub taken: u32,
    pub re_enable: u32,
    pub remove: u32,
    pub timed: u32,
    pub targeted_timed: u32,
    pub targeted: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub container: Container,
    pub ordinal: u32,
    pub token_start: usize,
    pub token_len: usize,
    pub days: i32,
    pub lifecycle: i32,
    pub target_kind: TargetKind,
    pub country_target: u32,
    pub numeric_target: u32,
    pub ignore: u8,
    pub description_seed: Option<Option<i32>>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RandomItemRow {
    pub country: u32,
    pub ordinal: u32,
    pub token_start: usize,
    pub token_len: usize,
    pub count: i32,
    pub target: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RandomItemDemand {
    pub rows: usize,
    pub bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RandomItemWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

pub fn count_random_items(memory: &impl Memory, base: usize, state: usize) -> RandomItemWalk {
    walk_random_items_with(memory, base, state, Buffer::Count, Buffer::Count)
}

pub fn walk_random_items(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<RandomItemRow>,
    tokens: &mut Vec<u8>,
) -> RandomItemWalk {
    walk_random_items_with(
        memory,
        base,
        state,
        Buffer::Growing(output),
        Buffer::Growing(tokens),
    )
}

fn walk_random_items_with(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: Buffer<'_, RandomItemRow>,
    mut tokens: Buffer<'_, u8>,
) -> RandomItemWalk {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, state)?;
        let mut len = 0_usize;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0_u32;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let status = status(memory, base, country.pointer)?;
            let array = status
                .checked_add(RANDOM_ITEMS)
                .and_then(|at| Array::read(memory, at, RANDOM_ITEM_STRIDE))
                .ok_or("decision random-item array unreadable or invalid")?;
            output.ensure(
                len.checked_add(array.count as usize)
                    .ok_or("decision random-item row demand overflow")?,
            )?;
            for ordinal in 0..array.count {
                let at = array
                    .element(ordinal, RANDOM_ITEM_STRIDE)
                    .ok_or("decision random-item address overflow")?;
                let entry = read::<[u8; RANDOM_ITEM_STRIDE]>(memory, at)
                    .ok_or("decision random-item entry unreadable")?;
                if engine_sites::at(base, RANDOM_ITEM_VTABLE_RVA).map(|p| p as u64)
                    != Some(u64::from_le_bytes(entry[..8].try_into().unwrap()))
                {
                    return Err("decision random-item vtable mismatch");
                }
                let definition =
                    usize::try_from(u64::from_le_bytes(entry[8..16].try_into().unwrap()))
                        .map_err(|_| "decision random-item definition address overflow")?;
                let token = cdecision::token(memory, base, definition)?;
                required_bytes = required_bytes
                    .checked_add(token.len)
                    .ok_or("decision random-item token demand overflow")?;
                let token_end = bytes_len
                    .checked_add(token.len)
                    .ok_or("decision random-item token span overflow")?;
                tokens.ensure(token_end)?;
                if len == output.len() || token_end > tokens.len() {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("decision random-item omitted count overflow")?;
                    continue;
                }
                if !read_slice(
                    memory,
                    token.address,
                    &mut tokens[bytes_len..bytes_len + token.len],
                ) {
                    return Err("decision random-item token bytes unreadable");
                }
                output[len] = RandomItemRow {
                    country: country.id,
                    ordinal,
                    token_start: bytes_len,
                    token_len: token.len,
                    count: i32::from_le_bytes(entry[16..20].try_into().unwrap()),
                    target: i32::from_le_bytes(entry[20..24].try_into().unwrap()),
                };
                len += 1;
                bytes_len += token.len;
            }
        }
        Ok(RandomItemWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(|reason| RandomItemWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

pub fn random_item_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<RandomItemDemand, &'static str> {
    let walk = count_random_items(memory, base, state);
    match walk.outcome {
        Outcome::Unavailable { reason } => Err(reason),
        Outcome::Rows { len } => Ok(RandomItemDemand {
            rows: len,
            bytes: walk.required_bytes,
        }),
        Outcome::Truncated { len, unread_rows } => Ok(RandomItemDemand {
            rows: len
                .checked_add(unread_rows as usize)
                .ok_or("decision random-item row demand overflow")?,
            bytes: walk.required_bytes,
        }),
    }
}

fn status(memory: &impl Memory, base: usize, country: usize) -> Result<usize, &'static str> {
    let expected =
        engine_sites::at(base, ccountry::VTABLE_RVA).ok_or("decision country vtable mismatch")?;
    if read::<u64>(memory, country) != Some(expected as u64) {
        return Err("decision country vtable mismatch");
    }
    let status = country
        .checked_add(ccountry::DECISION_STATUS)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("decision status pointer unreadable or null")?;
    let identity = read::<[u8; 16]>(memory, status).ok_or("decision status identity unreadable")?;
    if engine_sites::at(base, VTABLE_RVA).map(|p| p as u64)
        != Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
    {
        return Err("decision status vtable mismatch");
    }
    if u64::from_le_bytes(identity[8..].try_into().unwrap()) != country as u64 {
        return Err("decision status owner mismatch");
    }
    Ok(status)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub counts: Option<Counts>,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

pub fn count(memory: &impl Memory, base: usize, country: usize) -> Walk {
    walk_buffer(memory, base, country, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    country: usize,
    output: &mut Vec<Row>,
    tokens: &mut Vec<u8>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        country,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(tokens),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    country: usize,
    output: engine_native_memory::walk::Buffer<'_, Row>,
    tokens: engine_native_memory::walk::Buffer<'_, u8>,
) -> Walk {
    walk_inner(memory, base, country, output, tokens, None)
}

pub fn walk_with_description_demand_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    country: usize,
    output: engine_native_memory::walk::Buffer<'_, Row>,
    tokens: engine_native_memory::walk::Buffer<'_, u8>,
    demand: crate::border_effect_demand::Demand<'_>,
) -> Walk {
    walk_inner(
        memory,
        base,
        country,
        output,
        tokens,
        Some((game_state, demand)),
    )
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    country: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut tokens: engine_native_memory::walk::Buffer<'_, u8>,
    demand: Option<(usize, crate::border_effect_demand::Demand<'_>)>,
) -> Walk {
    let mut counts = None;
    let result = (|| {
        let status = status(memory, base, country)?;
        let array = |offset| {
            let header = status
                .checked_add(offset)
                .ok_or("decision vector address overflow")?;
            Array::read(memory, header, RECORD_POINTER_STRIDE)
                .ok_or("decision vector header unreadable or invalid")
        };
        let taken = array(DECISIONS_TAKEN)?;
        let re_enable = array(RE_ENABLE)?;
        let remove = array(REMOVE)?;
        let timed = array(ACTIVE_TIMED)?;
        let targeted_timed = array(TARGETED_TIMED)?;
        let targeted = array(TARGETED)?;
        counts = Some(Counts {
            taken: taken.count,
            re_enable: re_enable.count,
            remove: remove.count,
            timed: timed.count,
            targeted_timed: targeted_timed.count,
            targeted: targeted.count,
        });
        let row_demand = [re_enable, remove, timed, targeted_timed, targeted]
            .iter()
            .try_fold(0_usize, |total, array| {
                total
                    .checked_add(array.count as usize)
                    .ok_or("decision row demand overflow")
            })?;
        output.ensure(row_demand)?;
        let mut len = 0;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0_u32;
        let mut random_items = None;
        let mut countries = None;
        for (container, array) in [
            (Container::ReEnable, re_enable),
            (Container::Remove, remove),
            (Container::Timed, timed),
            (Container::Targeted, targeted),
            (Container::TargetedTimed, targeted_timed),
        ] {
            for ordinal in 0..array.count {
                let address = array
                    .element(ordinal, RECORD_POINTER_STRIDE)
                    .ok_or("decision row address overflow")?;
                let pointer = read::<u64>(memory, address)
                    .and_then(|value| usize::try_from(value).ok())
                    .filter(|&value| value != 0)
                    .ok_or("decision row pointer unreadable or null")?;
                let mut row = Row {
                    container,
                    ordinal,
                    ..Row::default()
                };
                let definition = match container {
                    Container::ReEnable | Container::Remove => {
                        let value = cdecision_cooldown::record(memory, base, pointer)?;
                        row.days = value.days;
                        value.definition
                    }
                    Container::Timed => {
                        let value = ctimed_decision::record(memory, base, pointer)?;
                        row.days = value.days;
                        row.lifecycle = value.lifecycle;
                        value.definition
                    }
                    Container::TargetedTimed | Container::Targeted => {
                        let value = ctargeted_decision::record(memory, base, pointer)?;
                        row.days = value.days;
                        row.lifecycle = value.lifecycle;
                        row.ignore = value.ignore;
                        row.country_target = value.country_target;
                        row.numeric_target = value.numeric_target;
                        row.target_kind = TargetKind::from_country_reference(value.country_target);
                        value.definition
                    }
                };
                let token = cdecision::token(memory, base, definition)?;
                required_bytes = required_bytes
                    .checked_add(token.len)
                    .ok_or("decision token byte count overflow")?;
                tokens.ensure(required_bytes)?;
                if len == output.len() || token.len > tokens.len() - bytes_len {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("decision omitted count overflow")?;
                    continue;
                }
                if !read_slice(
                    memory,
                    token.address,
                    &mut tokens[bytes_len..bytes_len + token.len],
                ) {
                    return Err("decision token bytes unreadable");
                }
                row.token_start = bytes_len;
                row.token_len = token.len;
                if matches!(container, Container::Targeted | Container::TargetedTimed)
                    && let Some((game_state, demand)) = demand
                {
                    let key = &tokens[bytes_len..bytes_len + token.len];
                    let mut demanded = false;
                    for &effect in demand.names {
                        if crate::border_effect_demand::matches_name(effect, demand.bytes, key)? {
                            demanded = true;
                            break;
                        }
                    }
                    if demanded {
                        let fixed = definition
                            .checked_add(cdecision::FIXED_RANDOM_SEED)
                            .and_then(|at| read::<u8>(memory, at))
                            .ok_or("decision fixed-random-seed flag unreadable")?;
                        if fixed == 0 {
                            let target = if (row.country_target as i32) > 0 {
                                if countries.is_none() {
                                    countries =
                                        Some(ccountry::Reader::new(memory, base, game_state)?);
                                }
                                countries
                                    .as_ref()
                                    .unwrap()
                                    .resolve(row.country_target)?
                                    .canonical_slot as i32
                            } else {
                                row.numeric_target as i32
                            };
                            let items = match random_items {
                                Some(items) => items,
                                None => {
                                    let items = Array::read(
                                        memory,
                                        status
                                            .checked_add(RANDOM_ITEMS)
                                            .ok_or("decision seed header overflow")?,
                                        RANDOM_ITEM_STRIDE,
                                    )
                                    .ok_or("decision seed header unreadable or invalid")?;
                                    random_items = Some(items);
                                    items
                                }
                            };
                            row.description_seed = Some(description_seed(
                                memory, base, items, definition, key, target,
                            )?);
                        }
                    }
                }
                output[len] = row;
                len += 1;
                bytes_len += token.len;
            }
        }
        Ok(Walk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            counts,
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(|reason| Walk {
        outcome: Outcome::Unavailable { reason },
        counts,
        bytes_len: 0,
        required_bytes: 0,
    })
}

fn description_seed(
    memory: &impl Memory,
    base: usize,
    items: Array,
    definition: usize,
    key: &[u8],
    target: i32,
) -> Result<Option<i32>, &'static str> {
    for ordinal in 0..items.count {
        let at = items
            .element(ordinal, RANDOM_ITEM_STRIDE)
            .ok_or("decision seed row overflow")?;
        let bytes =
            read::<[u8; RANDOM_ITEM_STRIDE]>(memory, at).ok_or("decision seed row unreadable")?;
        if engine_sites::at(base, RANDOM_ITEM_VTABLE_RVA).map(|p| p as u64)
            != Some(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
        {
            return Err("decision seed row vtable mismatch");
        }
        let candidate = usize::try_from(u64::from_le_bytes(bytes[8..16].try_into().unwrap()))
            .map_err(|_| "decision seed definition pointer overflow")?;
        if candidate == 0 {
            return Err("decision seed definition is null");
        }
        let same = if candidate == definition {
            true
        } else {
            let token = cdecision::token(memory, base, candidate)?;
            crate::border_effect_demand::matches(
                memory,
                crate::border_effect_demand::Token {
                    address: token.address,
                    len: token.len,
                },
                key,
            )?
        };
        if same && i32::from_le_bytes(bytes[RANDOM_ITEM_TARGET..].try_into().unwrap()) == target {
            return Ok(Some(i32::from_le_bytes(
                bytes[RANDOM_ITEM_SEED..RANDOM_ITEM_TARGET]
                    .try_into()
                    .unwrap(),
            )));
        }
    }
    Ok(None)
}

pub const HOURLY_UPDATE_RVA: Rva = Rva(0x0077_7e30);

#[cfg(test)]
mod tests;
