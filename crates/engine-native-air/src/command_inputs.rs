use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, msvc_string::Descriptor, read};
use engine_native::{ccountry, cgame_state, chuman};

pub struct Site {
    pub rva: Rva,
    pub span: usize,
}

pub const SITES: [Site; 10] = [
    Site {
        rva: Rva(0x022c_64a4),
        span: 5,
    },
    Site {
        rva: Rva(0x022c_6d07),
        span: 6,
    },
    Site {
        rva: Rva(0x022c_4847),
        span: 6,
    },
    Site {
        rva: Rva(0x00e4_8dd9),
        span: 7,
    },
    Site {
        rva: Rva(0x00e4_79c2),
        span: 10,
    },
    Site {
        rva: Rva(0x00e4_ae94),
        span: 7,
    },
    Site {
        rva: Rva(0x00e4_b6a6),
        span: 5,
    },
    Site {
        rva: Rva(0x00e4_cac5),
        span: 7,
    },
    Site {
        rva: Rva(0x00e4_e17d),
        span: 6,
    },
    Site {
        rva: Rva(0x00e4_e643),
        span: 7,
    },
];

pub const SESSION_STATE: usize = 0x30;
pub const SESSION_PLAYER: usize = 0x84;
pub const REMOVED_PLAYER_KEY: usize = 0x2c;
const HUMAN_COUNTRY_AUTOMATION_RVA: Rva = Rva(0x033d_daa9);
pub const IDLER_RVA: Rva = Rva(0x033d_db08);
pub const IN_GAME_IDLER_VTABLE_RVA: Rva = Rva(0x029e_58d8);
pub const IDLER_PAUSED: usize = 0x6c1;

pub fn idler_sites(base: usize) -> Result<(), &'static str> {
    engine_sites::require(base, &[IDLER_RVA, IN_GAME_IDLER_VTABLE_RVA])
}

pub fn in_game_idler(memory: &impl Memory, base: usize) -> Result<Option<usize>, &'static str> {
    let slot = engine_sites::at(base, IDLER_RVA).ok_or("command pause state unreadable")?;
    let in_game = engine_sites::at(base, IN_GAME_IDLER_VTABLE_RVA)
        .ok_or("command pause state unreadable")? as u64;
    let Some(idler) = read::<u64>(memory, slot).filter(|&pointer| pointer != 0) else {
        return Ok(None);
    };
    let idler = idler as usize;
    Ok((read::<u64>(memory, idler) == Some(in_game)).then_some(idler))
}

pub fn idler_paused(memory: &impl Memory, idler: usize) -> Result<bool, &'static str> {
    let at = idler
        .checked_add(IDLER_PAUSED)
        .ok_or("command pause state unreadable")?;
    match read::<u8>(memory, at) {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err("command pause state unreadable"),
    }
}

pub fn paused(memory: &impl Memory, base: usize) -> Result<Option<bool>, &'static str> {
    match in_game_idler(memory, base)? {
        Some(idler) => idler_paused(memory, idler).map(Some),
        None => Ok(None),
    }
}

pub fn country_tag(
    memory: &impl Memory,
    base: usize,
    state: usize,
    country: u32,
) -> Result<Option<Descriptor>, &'static str> {
    if country == 0 {
        return Ok(None);
    }
    let country = ccountry::Reader::new(memory, base, state)?.resolve(country)?;
    let tags = Array::read(
        memory,
        state + cgame_state::COUNTRY_TAGS,
        engine_native_memory::msvc_string::STRIDE,
    )
    .ok_or("command country tags unreadable")?;
    let at = tags
        .element(country.id, engine_native_memory::msvc_string::STRIDE)
        .ok_or("command country tag absent")?;
    Descriptor::read(memory, at).map(Some)
}

pub fn manual_country(
    memory: &impl Memory,
    base: usize,
    state: usize,
    command_country: u32,
    actor_country: u32,
) -> Result<bool, &'static str> {
    if engine_sites::at(base, HUMAN_COUNTRY_AUTOMATION_RVA)
        .and_then(|address| read::<u8>(memory, address))
        .ok_or("human-country automation mode unreadable")?
        != 0
    {
        return Ok(false);
    }
    if command_country == 0 || actor_country == 0 {
        return Ok(false);
    }
    if command_country == actor_country {
        return Ok(true);
    }
    let countries = ccountry::Reader::new(memory, base, state)?;
    Ok(countries.resolve(command_country)?.canonical_slot
        == countries.resolve(actor_country)?.canonical_slot)
}

#[derive(Clone, Copy, Debug)]
pub struct Header {
    pub vtable: usize,
    pub player: i32,
    pub sequence: u32,
}

pub fn header(memory: &impl Memory, command: usize) -> Result<Header, &'static str> {
    let bytes = read::<[u8; 36]>(memory, command).ok_or("command envelope unreadable")?;
    Ok(Header {
        vtable: usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
            .map_err(|_| "command vtable overflow")?,
        player: i32::from_le_bytes(bytes[12..16].try_into().unwrap()),
        sequence: u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
    })
}

#[derive(Clone, Copy)]
pub struct Player {
    pub key: i32,
    pub country: u32,
    pub user: Descriptor,
    pub name: Descriptor,
}

pub fn player(
    memory: &impl Memory,
    base: usize,
    state: usize,
    key: i32,
) -> Result<Option<Player>, &'static str> {
    if key < 0 {
        return Ok(None);
    }
    let players = Array::read(
        memory,
        state
            .checked_add(cgame_state::PLAYER_RECORDS)
            .ok_or("player roster overflow")?,
        chuman::STRIDE,
    )
    .ok_or("player roster unreadable")?;
    let vtable =
        engine_sites::at(base, chuman::VTABLE_RVA).ok_or("player identity overflow")? as u64;
    for index in 0..players.count {
        let pointer = players
            .element(index, chuman::STRIDE)
            .ok_or("player row overflow")?;
        let actual =
            read::<i32>(memory, pointer + chuman::PLAYER_KEY).ok_or("player key unreadable")?;
        if actual != key {
            continue;
        }
        if read::<u64>(memory, pointer) != Some(vtable) {
            return Err("player identity mismatch");
        }
        return player_at(memory, base, pointer).map(Some);
    }
    Ok(None)
}

pub fn player_at(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Player, &'static str> {
    let vtable = engine_sites::at(base, chuman::VTABLE_RVA).ok_or("player identity mismatch")?;
    if read::<u64>(memory, pointer) != Some(vtable as u64) {
        return Err("player identity mismatch");
    }
    Ok(Player {
        key: read(memory, pointer + chuman::PLAYER_KEY).ok_or("player key unreadable")?,
        country: read(memory, pointer + chuman::COUNTRY_TAG).ok_or("player country unreadable")?,
        user: Descriptor::read(memory, pointer + chuman::FIRST_STRING)?,
        name: Descriptor::read(memory, pointer + chuman::NAME)?,
    })
}

pub fn autonomous(
    memory: &impl Memory,
    base: usize,
    state: usize,
    raw: u32,
) -> Result<bool, &'static str> {
    let reader = ccountry::Reader::new(memory, base, state)?;
    let canonical = if raw == 0 {
        0
    } else {
        reader.resolve(raw)?.canonical_slot
    };
    let counts = Array::read(memory, state + cgame_state::HUMAN_STATUS, 1)
        .ok_or("command human control header invalid")?;
    if canonical < counts.count {
        let count = read::<i8>(
            memory,
            counts
                .element(canonical, 1)
                .ok_or("command control slot overflow")?,
        )
        .ok_or("command human control unreadable")?;
        if count < 0 {
            return Err("command human controller count negative");
        }
        let flags = Array::read(memory, state + cgame_state::AUTONOMOUS_STATUS, 1)
            .ok_or("command autonomous control header invalid")?;
        let flag = flags
            .element(canonical, 1)
            .and_then(|pointer| read::<u8>(memory, pointer))
            .ok_or("command autonomous control unreadable")?;
        return Ok(flag != 0 && count == 0);
    }
    let players = Array::read(memory, state + cgame_state::PLAYER_RECORDS, chuman::STRIDE)
        .ok_or("command fallback roster unreadable")?;
    let vtable = engine_sites::at(base, chuman::VTABLE_RVA)
        .ok_or("command fallback player identity mismatch")? as u64;
    for index in 0..players.count {
        let at = players
            .element(index, chuman::STRIDE)
            .ok_or("command fallback player overflow")?;
        if read::<u64>(memory, at) != Some(vtable) {
            return Err("command fallback player identity mismatch");
        }
        let country = read::<u32>(memory, at + chuman::COUNTRY_TAG)
            .ok_or("command fallback country unreadable")?;
        if country == raw {
            return Ok(false);
        }
        if country != 0 && raw != 0 && reader.resolve(country)?.canonical_slot == canonical {
            return Ok(false);
        }
    }
    Ok(true)
}

mod owners;
pub use owners::{Inputs, Resolution, target};
