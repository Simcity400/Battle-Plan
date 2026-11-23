
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_allocation as allocation;

pub const ACCOUNTS: usize = 0x08;
pub const STRIDE: usize = 0xd0;
pub const COUNTRY: usize = 0;
pub const INFLUENCE: usize = 0x08;
pub const POWER_PROJECTION: usize = 0x20;
pub const INFLUENCE_SHARE: usize = 0x38;
pub const POWER_PROJECTION_SHARE: usize = 0x40;
pub const INITIATIVE: usize = 0x48;
pub const POWER_PROJECTION_RANK: usize = 0x50;
pub const UPGRADES: usize = 0x58;
pub const CONTRIBUTION: usize = 0x78;
pub const CONTRIBUTION_GAIN: usize = 0x80;
pub const PREVIOUS_WAR_SCORE: usize = 0xb0;
pub const COMPLETED_GOALS: usize = 0xb8;
pub const UPGRADE_VTABLE_RVA: Rva = Rva(0x0279_7c10);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Upgrade {
    pub group: u32,
    pub token: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub country: u32,
    pub influence: Vec<i64>,
    pub power_projection: Vec<i64>,
    pub influence_share_raw: i64,
    pub power_projection_share_raw: i64,
    pub initiative_raw: i64,
    pub power_projection_rank: i32,
    pub contribution_raw: i64,
    pub contribution_gain: [i64; 2],
    pub previous_war_score_raw: i64,
    pub upgrades: Vec<Upgrade>,
    pub completed_goals: Vec<Option<u32>>,
}

fn at(address: usize, offset: usize) -> Result<usize, &'static str> {
    address
        .checked_add(offset)
        .ok_or("faction member account address overflow")
}
fn word<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    address: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(address, offset)?).ok_or("faction member account field unreadable")
}
fn values(memory: &impl Memory, address: usize, output: &mut Vec<i64>) -> Result<(), &'static str> {
    let array = Array::read(memory, address, 8)
        .ok_or("faction member statistic array unreadable or invalid")?;
    allocation::resize_default(output, array.count as usize)
        .map_err(|_| "faction member statistics allocation failed")?;
    for (ordinal, value) in output.iter_mut().enumerate() {
        *value = read(
            memory,
            array
                .element(ordinal as u32, 8)
                .ok_or("faction member statistic address overflow")?,
        )
        .ok_or("faction member statistic unreadable")?;
    }
    Ok(())
}

pub fn read_upgrades(
    memory: &impl Memory,
    base: usize,
    table: usize,
    output: &mut Vec<Upgrade>,
) -> Result<(), &'static str> {
    const STORAGE: usize = 0x08;
    const COUNT: usize = 0x10;
    const MASK: usize = 0x14;
    const TAIL: usize = 0x18;
    const NODE_STRIDE: usize = 0x18;
    const DISTANCE: usize = 4;
    const GROUP: usize = 8;
    const VALUE: usize = 0x10;
    const TOKEN: usize = 8;
    let count: u32 = word(memory, table, COUNT)?;
    if count == 0 {
        output.clear();
        return Ok(());
    }
    let storage = usize::try_from(word::<u64>(memory, table, STORAGE)?)
        .map_err(|_| "faction upgrade storage invalid")?;
    let mask: u32 = word(memory, table, MASK)?;
    let tail: u8 = word(memory, table, TAIL)?;
    let buckets = mask
        .checked_add(1)
        .filter(|n| n.is_power_of_two())
        .ok_or("faction upgrade bucket geometry invalid")?;
    let slots = buckets
        .checked_add(u32::from(tail))
        .ok_or("faction upgrade slot count overflow")?;
    if storage == 0 || count > slots {
        return Err("faction upgrade selection geometry invalid");
    }
    at(
        storage,
        (slots as usize)
            .checked_mul(NODE_STRIDE)
            .ok_or("faction upgrade storage overflow")?,
    )?;
    allocation::resize_default(output, count as usize)
        .map_err(|_| "faction upgrade selection allocation failed")?;
    let mut len = 0;
    for ordinal in 0..slots {
        let node = at(storage, ordinal as usize * NODE_STRIDE)?;
        let distance: u8 = word(memory, node, DISTANCE)?;
        if distance == 0 {
            continue;
        }
        if distance == u8::MAX {
            return Err("faction upgrade selection ended before its count");
        }
        if len == output.len() {
            return Err("faction upgrade selection exceeds its count");
        }
        let group = word(memory, node, GROUP)?;
        let definition = usize::try_from(word::<u64>(memory, node, VALUE)?)
            .map_err(|_| "faction member upgrade pointer invalid")?;
        let vtable = engine_sites::at(base, UPGRADE_VTABLE_RVA)
            .ok_or("faction member upgrade definition vtable mismatch")?;
        if definition == 0 || read::<u64>(memory, definition) != Some(vtable as u64) {
            return Err("faction member upgrade definition vtable mismatch");
        }
        output[len] = Upgrade {
            group,
            token: word(memory, definition, TOKEN)?,
        };
        len += 1;
        if len == output.len() {
            break;
        }
    }
    if len != output.len() {
        return Err("faction upgrade selection is short of its count");
    }
    Ok(())
}

pub fn read_accounting_into(
    memory: &impl Memory,
    base: usize,
    accounts: Array,
    index: u32,
    output: &mut Snapshot,
) -> Result<(), &'static str> {
    let address = accounts
        .element(index, STRIDE)
        .ok_or("faction member account outside its array")?;
    output.country = word(memory, address, COUNTRY)?;
    values(memory, at(address, INFLUENCE)?, &mut output.influence)?;

    output.initiative_raw = word(memory, address, INITIATIVE)?;
    output.contribution_raw = word(memory, address, CONTRIBUTION)?;
    for (ordinal, value) in output.contribution_gain.iter_mut().enumerate() {
        *value = word(
            memory,
            address,
            CONTRIBUTION_GAIN + ordinal * std::mem::size_of::<i64>(),
        )?;
    }
    output.previous_war_score_raw = word(memory, address, PREVIOUS_WAR_SCORE)?;
    read_selections_into(memory, base, accounts, index, output)
}

pub fn read_selections_into(
    memory: &impl Memory,
    base: usize,
    accounts: Array,
    index: u32,
    output: &mut Snapshot,
) -> Result<(), &'static str> {
    let address = accounts
        .element(index, STRIDE)
        .ok_or("faction member account outside its array")?;
    read_upgrades(memory, base, at(address, UPGRADES)?, &mut output.upgrades)?;
    let goals = Array::read(memory, at(address, COMPLETED_GOALS)?, 8)
        .ok_or("faction member completed goals unreadable or invalid")?;
    allocation::resize_default(&mut output.completed_goals, goals.count as usize)
        .map_err(|_| "faction member completed goals allocation failed")?;
    for (ordinal, value) in output.completed_goals.iter_mut().enumerate() {
        let definition = usize::try_from(
            read::<u64>(
                memory,
                goals
                    .element(ordinal as u32, 8)
                    .ok_or("faction member goal address overflow")?,
            )
            .ok_or("faction member goal pointer unreadable")?,
        )
        .map_err(|_| "faction member goal pointer invalid")?;
        *value = if definition == 0 {
            None
        } else {
            let vtable =
                engine_sites::at(base, crate::cfaction_goal_status::DEFINITION_VTABLE_RVA)
                    .ok_or("faction member completed goal definition vtable mismatch")?;
            if read::<u64>(memory, definition) != Some(vtable as u64) {
                return Err("faction member completed goal definition vtable mismatch");
            }
            Some(word(
                memory,
                definition,
                crate::cfaction_goal_status::DEFINITION_TOKEN,
            )?)
        };
    }
    Ok(())
}

pub fn read_into(
    memory: &impl Memory,
    base: usize,
    accounts: Array,
    index: u32,
    output: &mut Snapshot,
) -> Result<(), &'static str> {
    read_accounting_into(memory, base, accounts, index, output)?;
    let address = accounts
        .element(index, STRIDE)
        .ok_or("faction member account outside its array")?;
    values(
        memory,
        at(address, POWER_PROJECTION)?,
        &mut output.power_projection,
    )?;
    output.influence_share_raw = word(memory, address, INFLUENCE_SHARE)?;
    output.power_projection_share_raw = word(memory, address, POWER_PROJECTION_SHARE)?;
    output.power_projection_rank = word(memory, address, POWER_PROJECTION_RANK)?;
    Ok(())
}

#[cfg(test)]
mod tests;

pub const PROJECTION_FACTORS: [Rva; 7] = [
    Rva(0x33e4be0),
    Rva(0x33e4cf0),
    Rva(0x33e4dc8),
    Rva(0x33e5098),
    Rva(0x33e5168),
    Rva(0x33e4ec0),
    Rva(0x33e4f90),
];
pub fn projection_factors(memory: &impl Memory, base: usize) -> Result<[i64; 7], &'static str> {
    let mut values = [0; 7];
    for (target, offset) in values.iter_mut().zip(PROJECTION_FACTORS) {
        *target = read(
            memory,
            engine_sites::at(base, offset)
                .ok_or("faction projection coefficient address overflow")?,
        )
        .ok_or("faction projection coefficient unreadable")?;
    }
    Ok(values)
}
