
use engine_native_memory::{Memory, msvc_string::Literal, read};
use engine_native::{ccountry, cgame_state};
use engine_native_intelligence::{coperation, coperation_instance as operation};
use engine_sites::Rva;

pub const COMPLETION_RVA: Rva = Rva(0x0149_f710);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub owner: u32,
    pub id: u64,
    pub target: u32,
    pub definition_token: u32,
    pub completion_hour: i32,
    pub terminal: u8,
    pub bonus: u8,
    pub detected: u8,
}

pub struct Observation {
    pub row: Row,
    description: Literal,
}
impl Observation {
    pub fn description_len(&self) -> usize {
        self.description.len
    }
    pub fn copy_description(
        &self,
        memory: &impl Memory,
        output: &mut [u8],
    ) -> Result<(), &'static str> {
        Literal::copy_all(memory, [&self.description], output)
            .map_err(|_| "operation completion description bytes unreadable")
    }
    pub fn copy_description_parts(
        &self,
        memory: &impl Memory,
        first: &mut [u8],
        second: &mut [u8],
    ) -> Result<(), &'static str> {
        const UNREADABLE: &str = "operation completion description bytes unreadable";
        if first.len().checked_add(second.len()) != Some(self.description.len) {
            return Err(UNREADABLE);
        }
        if second.is_empty() {
            return self.copy_description(memory, first);
        }
        if self.description.len <= 15 {
            let mut inline = [0; 15];
            self.copy_description(memory, &mut inline[..self.description.len])?;
            first.copy_from_slice(&inline[..first.len()]);
            second.copy_from_slice(&inline[first.len()..self.description.len]);
            return Ok(());
        }
        let source = self.description.source();
        let tail = source.checked_add(first.len()).ok_or(UNREADABLE)?;
        if memory.copy_scattered([source, tail], &mut [first, second]) != 2 {
            return Err(UNREADABLE);
        }
        Ok(())
    }
}

pub fn observe(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Observation, &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("operation completion address overflow")
    };
    let expected = engine_sites::at(base, operation::VTABLE_RVA)
        .ok_or("operation completion module overflow")?;
    let identity =
        read::<[u8; 16]>(memory, pointer).ok_or("operation completion identity unreadable")?;
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64 {
        return Err("operation completion instance vtable mismatch");
    }
    let id = u64::from_le_bytes(identity[8..].try_into().unwrap());
    if id == 0 {
        return Err("operation completion null handle");
    }
    let definition_pointer = read::<u64>(memory, at(operation::DEFINITION)?)
        .ok_or("operation completion definition unreadable")? as usize;
    let definition_token = coperation::definition(memory, base, definition_pointer)?;
    let owner_pointer = read::<u64>(memory, at(operation::OWNER_COUNTRY)?)
        .ok_or("operation completion owner unreadable")? as usize;
    let owner = read::<[u8; 16]>(memory, owner_pointer)
        .ok_or("operation completion owner identity unreadable")?;
    if u64::from_le_bytes(owner[..8].try_into().unwrap())
        != engine_sites::at(base, ccountry::VTABLE_RVA)
            .ok_or("operation completion module overflow")? as u64
    {
        return Err("operation completion owner vtable mismatch");
    }
    let owner = u32::from_le_bytes(owner[ccountry::ID..ccountry::ID + 4].try_into().unwrap());
    if owner == 0 {
        return Err("operation completion null owner");
    }
    let target = read::<u32>(memory, at(operation::TARGET_COUNTRY_ID)?)
        .ok_or("operation completion target unreadable")?;
    let terminal = read::<u8>(memory, at(operation::TERMINAL)?)
        .ok_or("operation completion terminal unreadable")?;
    let outcome = read::<[u8; 34]>(memory, at(operation::EFFECT_DESCRIPTION)?)
        .ok_or("operation completion outcomes unreadable")?;
    if terminal != 1 || outcome[32..].iter().any(|&flag| flag > 1) {
        return Err("operation completion stored flags invalid");
    }
    let header = outcome[..32].try_into().unwrap();
    if u64::from_le_bytes(outcome[16..24].try_into().unwrap()) > operation::DESCRIPTION_MAX_LENGTH
        || u64::from_le_bytes(outcome[24..32].try_into().unwrap())
            > operation::DESCRIPTION_MAX_LENGTH
    {
        return Err("operation completion description exceeds native string maximum");
    }
    let description = Literal::from_header(at(operation::EFFECT_DESCRIPTION)?, header)
        .map_err(|_| "operation completion description header invalid")?;
    Literal::validate(memory, [&description])
        .map_err(|_| "operation completion description terminator unreadable or invalid")?;
    let state = read::<u64>(
        memory,
        engine_sites::at(base, cgame_state::SINGLETON_RVA)
            .ok_or("operation completion clock address overflow")?,
    )
    .ok_or("operation completion clock unreadable")? as usize;
    let completion_hour = read::<i32>(
        memory,
        state
            .checked_add(cgame_state::CURRENT_HOUR)
            .ok_or("operation completion hour address overflow")?,
    )
    .ok_or("operation completion hour unreadable")?;
    Ok(Observation {
        row: Row {
            owner,
            id,
            target,
            definition_token,
            completion_hour,
            terminal,
            bonus: outcome[32],
            detected: outcome[33],
        },
        description,
    })
}
