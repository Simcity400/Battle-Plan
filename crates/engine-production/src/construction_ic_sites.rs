
use engine_native_memory::{Memory, cpdx_array::Array, read};
use crate::{
    cbuilding_production_line,
    cgeneral_production_line,
    crailway_production_line,
};
use engine_native::{cprovince::buildings as cbuilding_status, cgame_state};
use engine_native_parts::cproduction_line;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    pub country: u32,
    pub building_type: u32,
    pub target_flags: u32,
    pub target_location: u32,
    pub target_state: u32,
    pub target_country: u32,
    pub route_first: Option<u32>,
    pub route_last: Option<u32>,
}

fn identity(
    memory: &impl Memory,
    base: usize,
    line: usize,
    railway: bool,
    status: u64,
    building: u64,
    game_state: Option<usize>,
) -> Result<Identity, &'static str> {
    let country = cbuilding_production_line::owning_country(memory, base, status)?;
    let mut result = Identity {
        country,
        ..Identity::default()
    };
    if railway {
        let route = crailway_production_line::route(memory, line)?;
        let endpoint = |index| crailway_production_line::route_province(memory, route, index);
        if route.count != 0 {
            result.route_first = Some(endpoint(0)?);
            result.route_last = if route.count == 1 {
                result.route_first
            } else {
                Some(endpoint(route.count - 1)?)
            };
        }
    } else {
        let state = game_state
            .or_else(|| {
                engine_sites::at(base, cgame_state::SINGLETON_RVA)
                    .and_then(|p| read::<u64>(memory, p))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
            })
            .ok_or("construction output game state unreadable or null")?;
        let locations = cbuilding_status::Locations::new(memory, state)?;
        let target = cbuilding_production_line::target_fields(memory, base, building, &locations)?;
        result.building_type = target.building_type;
        result.target_flags = target.identity.flags;
        result.target_location = target.identity.location;
        result.target_state = target.state;
        result.target_country = target.identity.country;
    }
    Ok(result)
}

fn building_identity(
    memory: &impl Memory,
    base: usize,
    line: usize,
    game_state: Option<usize>,
) -> Option<Identity> {
    let mut status = [0; 8];
    let mut building = [0; 8];
    if memory.copy_scattered(
        [
            line.checked_add(cproduction_line::PRODUCTION_STATUS)?,
            line.checked_add(cbuilding_production_line::BUILDING)?,
        ],
        &mut [&mut status, &mut building],
    ) != 2
    {
        return None;
    }
    identity(
        memory,
        base,
        line,
        false,
        u64::from_le_bytes(status),
        u64::from_le_bytes(building),
        game_state,
    )
    .ok()
}

pub fn events_per_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let countries = engine_native::ccountry::Reader::new(memory, base, game_state)?;
    let mut events = 0_usize;
    let expected = |rva| {
        engine_sites::at(base, rva)
            .map(|address| address as u64)
            .ok_or("construction output module address overflow")
    };
    let building = expected(cbuilding_production_line::VTABLE_RVA)?;
    let railway = expected(crailway_production_line::VTABLE_RVA)?;
    let repair = expected(cbuilding_production_line::RAILWAY_GUN_REPAIR_VTABLE_RVA)?;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let status = engine_native::cproduction_status::manager(memory, base, country)?;
        let lines = status
            .checked_add(engine_native::cproduction_status::CONSTRUCTION_LINES)
            .and_then(|address| engine_native_memory::cpdx_array::Array::read(memory, address, 8))
            .ok_or("construction output sizing queue unreadable or invalid")?;
        for ordinal in 0..lines.count {
            let pointer = lines
                .element(ordinal, 8)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|&pointer| pointer != 0)
                .ok_or("construction output sizing line unreadable or null")?;
            let vtable = read::<u64>(memory, pointer)
                .ok_or("construction output sizing line vtable unreadable")?;
            let demand = if vtable == building {
                3
            } else if vtable == railway {
                let edges = (crailway_production_line::route(memory, pointer)?.count as usize)
                    .saturating_sub(1);
                edges
                    .checked_add(1)
                    .ok_or("construction output railway event demand overflow")?
            } else if vtable == repair {
                0
            } else {
                return Err("construction output sizing line vtable unsupported");
            };
            events = events
                .checked_add(demand)
                .ok_or("construction output event demand overflow")?;
        }
    }
    Ok(events)
}

#[cfg(test)]
#[path = "construction_ic_sites/demand_tests.rs"]
mod demand_tests;

pub struct Site {
    pub name: &'static str,
    pub rva: Rva,
}

pub const BUILDING_SPEED: Site = Site {
    name: "building speed",
    rva: Rva(0x00ff_2540),
};
pub const BUILDING_SPEND_RETURN: Rva = Rva(0x00fe_edc7);

pub const BUILDING_COMPLETE: Site = Site {
    name: "building completion",
    rva: Rva(0x00ff_2640),
};

pub const BUILDING_RESET: Site = Site {
    name: "building count reset",
    rva: Rva(0x00fe_ea50),
};

pub fn count_reset(
    memory: &impl Memory,
    base: usize,
    line: usize,
) -> Result<(u64, i32, Option<Identity>), &'static str> {
    const UNSUPPORTED: &str = "construction count reset line vtable unreadable or unsupported";
    if line == 0 {
        return Err(UNSUPPORTED);
    }
    let expected = engine_sites::at(base, LINE_VTABLES[0]).ok_or(UNSUPPORTED)?;
    if read::<u64>(memory, line) != Some(expected as u64) {
        return Err(UNSUPPORTED);
    }
    let handle = line
        .checked_add(LINE_HANDLE)
        .and_then(|p| read::<u64>(memory, p))
        .ok_or("construction count reset handle unreadable")?;
    let count = line
        .checked_add(cproduction_line::AMOUNT)
        .and_then(|p| read::<i32>(memory, p))
        .ok_or("construction count reset amount unreadable")?;
    Ok((handle, count, building_identity(memory, base, line, None)))
}

pub const RAILWAY_COMPLETE: Site = Site {
    name: "railway completion",
    rva: Rva(0x01a9_58c0),
};

pub const COMPLETE_RETURN: Rva = Rva(0x00ed_861e);

pub const REMOVE_GENERAL_LINE: Site = Site {
    name: "linked building completion removal",
    rva: Rva(0x00ee_03b0),
};
pub const LINKED_COMPLETE_RETURN: Rva = Rva(0x00fe_f2ec);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Completion {
    pub line: u64,
    pub identity: Option<Identity>,
    pub hour: i32,
    pub cancelled: bool,
}

pub fn completion(
    memory: &impl Memory,
    base: usize,
    line: usize,
    railway: bool,
) -> Result<Completion, &'static str> {
    let mut bytes = [0; crailway_production_line::COMPLETION_FLAG + 1];
    let length = if railway {
        bytes.len()
    } else {
        LINE_HANDLE + 8
    };
    if line == 0 || !memory.copy(line, &mut bytes[..length]) {
        return Err("construction completion line unreadable");
    }
    let word = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
    if engine_sites::at(base, LINE_VTABLES[usize::from(railway)]) != Some(word(0) as usize) {
        return Err("construction completion line vtable unsupported");
    }
    let handle = word(LINE_HANDLE);
    if handle == 0 {
        return Err("construction completion line handle is null");
    }
    let state = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|p| *p != 0)
        .ok_or("construction completion game state unreadable")?;
    let hour = state
        .checked_add(engine_native::cgame_state::CURRENT_HOUR)
        .and_then(|p| read::<i32>(memory, p))
        .ok_or("construction completion hour unreadable")?;
    Ok(Completion {
        line: handle,
        identity: if railway {
            identity(
                memory,
                base,
                line,
                true,
                word(cproduction_line::PRODUCTION_STATUS),
                0,
                Some(state),
            )
            .ok()
        } else {
            building_identity(memory, base, line, Some(state))
        },
        hour,
        cancelled: railway && bytes[crailway_production_line::COMPLETION_FLAG] != 0,
    })
}

pub fn linked_completion(
    memory: &impl Memory,
    base: usize,
    status: usize,
    line: usize,
) -> Result<Completion, &'static str> {
    let event = completion(memory, base, line, false)?;
    let mut bytes = [0; cproduction_line::AMOUNT + 4];
    if !memory.copy(line, &mut bytes) {
        return Err("linked construction completion receiver unreadable");
    }
    let word = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
    let int = |offset| i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    if status == 0 || word(cproduction_line::PRODUCTION_STATUS) != status as u64 {
        return Err("linked construction completion owner mismatch");
    }
    if int(cproduction_line::AMOUNT) != 0 {
        return Err("linked construction completion amount is not zero");
    }
    let mut owner = [0; engine_native::cproduction_status::CONSTRUCTION_LINES_COUNT + 4];
    if !memory.copy(status, &mut owner) {
        return Err("linked construction completion owner unreadable");
    }
    let owner_word = |offset| u64::from_le_bytes(owner[offset..offset + 8].try_into().unwrap());
    if engine_sites::at(base, engine_native::cproduction_status::VTABLE_RVA) != Some(owner_word(0) as usize)
    {
        return Err("linked construction completion owner vtable unsupported");
    }
    let owner_int = |offset| i32::from_le_bytes(owner[offset..offset + 4].try_into().unwrap());
    let count = owner_int(engine_native::cproduction_status::CONSTRUCTION_LINES_COUNT);
    let capacity = owner_int(engine_native::cproduction_status::CONSTRUCTION_LINES_CAPACITY);
    let index = int(cproduction_line::PRIORITY);
    if index < 0 || index >= count || count > capacity {
        return Err("linked construction completion roster index invalid");
    }
    let slot = usize::try_from(owner_word(engine_native::cproduction_status::CONSTRUCTION_LINES))
        .ok()
        .filter(|p| *p != 0)
        .and_then(|p| {
            (index as usize)
                .checked_mul(engine_native::cproduction_status::CONSTRUCTION_POINTER_STRIDE)
                .and_then(|offset| p.checked_add(offset))
        })
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok());
    if slot != Some(line) {
        return Err("linked construction completion roster receiver mismatch");
    }
    Ok(event)
}

pub const RAILWAY_ADVANCE: Site = Site {
    name: "railway advance",
    rva: Rva(0x01a9_44c0),
};

pub const RAILWAY_PROGRESS: Site = Site {
    name: "railway progress",
    rva: Rva(0x00f0_5d60),
};

pub const INSTANT_CONSTRUCTION: Rva = Rva(0x033d_da98);
pub const RAILWAY_INSTANT_AMOUNT: i64 = 10_000_000_000;

pub fn railway_instant(
    memory: &impl Memory,
    base: usize,
    line: usize,
) -> Result<bool, &'static str> {
    let flag = engine_sites::at(base, INSTANT_CONSTRUCTION)
        .and_then(|address| read::<u8>(memory, address))
        .ok_or("construction output instant flag unreadable")?;
    if flag == 0 {
        return Ok(false);
    }
    let country = crate::line_output_sites::owning_country(
        memory,
        base,
        line,
        &[crailway_production_line::VTABLE_RVA],
    )?;
    railway_instant_country(memory, base, country)
}

pub fn railway_instant_owned(
    memory: &impl Memory,
    base: usize,
    country: u32,
) -> Result<bool, &'static str> {
    let flag = engine_sites::at(base, INSTANT_CONSTRUCTION)
        .and_then(|address| read::<u8>(memory, address))
        .ok_or("construction output instant flag unreadable")?;
    if flag == 0 {
        return Ok(false);
    }
    railway_instant_country(memory, base, country)
}

fn railway_instant_country(
    memory: &impl Memory,
    base: usize,
    country: u32,
) -> Result<bool, &'static str> {
    let state = engine_sites::at(base, cgame_state::SINGLETON_RVA)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("construction output instant viewing state unreadable or null")?;
    let [playing, observed] = cgame_state::local_player(memory, state)?;
    let viewing = if (playing as i32) > 0 {
        playing
    } else {
        observed
    };
    if viewing == country {
        return Ok(true);
    }
    if viewing == 0 || country == 0 {
        return Ok(false);
    }
    let links = state
        .checked_add(cgame_state::COUNTRY_LINKS)
        .and_then(|address| Array::read(memory, address, 4))
        .ok_or("construction output instant country links unreadable")?;
    let link = |raw: u32| {
        links
            .element(raw, 4)
            .and_then(|address| read::<u32>(memory, address))
            .ok_or("construction output instant country link unreadable")
    };
    Ok(link(viewing)? == link(country)?)
}
pub const INVALID_REFERENCE: Rva = Rva(0x033e_b2a0);

pub const LINE_HANDLE: usize = cproduction_line::REFERENCE_TYPE;
pub const CIC_BANK_DRAW: usize = cgeneral_production_line::CIC_BANK_DRAW;
pub const REPAIR_LEVELS: usize = cbuilding_production_line::REPAIR_LEVELS_REMAINING;
pub const LINKED_LINE: usize = cbuilding_production_line::LINKED_LINE;

pub const LINE_VTABLES: [Rva; 2] = [
    cbuilding_production_line::VTABLE_RVA,
    crailway_production_line::VTABLE_RVA,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operands {
    pub line: u64,
    pub identity: Option<Identity>,
    pub bank: i64,
    pub cost: i64,
    pub speed: i64,
    pub repair: bool,
    pub partner: Option<u64>,
    pub instant: bool,
}

pub fn operands(
    memory: &impl Memory,
    base: usize,
    line: usize,
    railway: bool,
) -> Result<Operands, &'static str> {
    let mut bytes = [0; LINKED_LINE + 8];
    let length = if railway {
        CIC_BANK_DRAW + 8
    } else {
        bytes.len()
    };
    if line == 0 || !memory.copy(line, &mut bytes[..length]) {
        return Err("construction output line unreadable");
    }
    let word = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
    if engine_sites::at(base, LINE_VTABLES[usize::from(railway)]) != Some(word(0) as usize) {
        return Err("construction output line vtable unsupported");
    }
    let handle = word(LINE_HANDLE);
    if handle == 0 {
        return Err("construction output line handle is null");
    }
    let instant = !railway
        && engine_sites::at(base, INSTANT_CONSTRUCTION)
            .and_then(|p| read::<u8>(memory, p))
            .ok_or("construction output instant flag unreadable")?
            != 0;
    let invalid = if railway {
        0
    } else {
        engine_sites::at(base, INVALID_REFERENCE)
            .and_then(|p| read::<u64>(memory, p))
            .ok_or("construction output invalid reference unreadable")?
    };
    Ok(Operands {
        line: handle,
        identity: identity(
            memory,
            base,
            line,
            railway,
            word(cproduction_line::PRODUCTION_STATUS),
            if railway {
                0
            } else {
                word(cbuilding_production_line::BUILDING)
            },
            None,
        )
        .ok(),
        bank: word(CIC_BANK_DRAW) as i64,
        cost: word(cproduction_line::COST) as i64,
        speed: word(cproduction_line::SPEED) as i64,
        repair: !railway
            && i32::from_le_bytes(bytes[REPAIR_LEVELS..REPAIR_LEVELS + 4].try_into().unwrap()) > 0,
        partner: (!railway && word(LINKED_LINE) != invalid).then(|| word(LINKED_LINE)),
        instant,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_parts::cbuilding;
    use engine_native_memory::memory::SliceMemory;

    const BASE: usize = 0x140000000;
    const LINE: usize = 0x1000;
    const TARGET: usize = 0x20000;
    const TARGET_STATE: usize = 0x30000;
    const TARGET_COUNTRY: usize = 0x40000;

    fn captured_identity(railway: bool) -> Identity {
        if railway {
            Identity {
                country: 7,
                route_first: Some(3),
                route_last: Some(4),
                ..Identity::default()
            }
        } else {
            Identity {
                country: 7,
                building_type: 9,
                target_flags: 2,
                target_location: 1,
                target_state: 1,
                ..Identity::default()
            }
        }
    }

    fn target_regions(status: usize) -> Regions {
        let mut regions = Regions(Vec::new());
        let mut add = |at: usize, bytes: &[u8]| regions.0.push((at, bytes.to_vec()));
        add(
            BASE + cgame_state::SINGLETON_RVA.0,
            &0x2000_u64.to_le_bytes(),
        );
        add(
            status,
            &((BASE + engine_native::cproduction_status::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        add(
            status + engine_native::cproduction_status::COUNTRY,
            &(TARGET_COUNTRY as u64).to_le_bytes(),
        );
        let mut country = [0; 12];
        country[..8]
            .copy_from_slice(&((BASE + engine_native::ccountry::VTABLE_RVA.0) as u64).to_le_bytes());
        country[8..].copy_from_slice(&7_u32.to_le_bytes());
        add(TARGET_COUNTRY, &country);
        add(
            TARGET_COUNTRY + engine_native::ccountry::PRODUCTION_STATUS,
            &(status as u64).to_le_bytes(),
        );
        add(
            TARGET,
            &((BASE + cbuilding::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        add(TARGET + cbuilding::TYPE_ID, &9_u32.to_le_bytes());
        let building_status = TARGET_STATE + engine_native::cstate::BUILDING_STATUS;
        add(
            TARGET + cbuilding::BUILDING_STATUS,
            &(building_status as u64).to_le_bytes(),
        );
        add(
            building_status,
            &((BASE + cbuilding_status::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        let fields = [
            2_u32.to_le_bytes(),
            1_u32.to_le_bytes(),
            0_u32.to_le_bytes(),
        ]
        .concat();
        add(building_status + cbuilding_status::FLAGS, &fields);
        add(
            TARGET_STATE,
            &((BASE + engine_native::cstate::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        add(TARGET_STATE + engine_native::cstate::ID, &1_u32.to_le_bytes());
        let header = [
            0x50000_u64.to_le_bytes().to_vec(),
            2_u32.to_le_bytes().repeat(2),
        ]
        .concat();
        add(0x2000 + cgame_state::STATES, &header);
        add(0x2000 + cgame_state::PROVINCES, &header);
        add(0x50008, &(TARGET_STATE as u64).to_le_bytes());
        let route = [
            0x60000_u64.to_le_bytes().to_vec(),
            2_u32.to_le_bytes().repeat(2),
        ]
        .concat();
        add(LINE + crailway_production_line::ROUTE, &route);
        add(0x60000, &3_u32.to_le_bytes());
        add(0x60004, &4_u32.to_le_bytes());
        regions
    }

    struct WithTarget<M>(M);
    impl<M: Memory> Memory for WithTarget<M> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.0.copy(address, out) || target_regions(STATUS).copy(address, out)
        }
    }

    struct WithoutIdentity<'a, M>(&'a M);
    impl<M: Memory> Memory for WithoutIdentity<'_, M> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            address != STATUS
                && address != LINE + cproduction_line::PRODUCTION_STATUS
                && self.0.copy(address, out)
        }
    }

    #[test]
    fn count_reset_observes_the_native_signed_count_before_it_is_cleared() {
        let mut bytes = vec![0; LINE + cbuilding_production_line::BUILDING + 8];
        bytes[LINE..LINE + 8].copy_from_slice(&((BASE + LINE_VTABLES[0].0) as u64).to_le_bytes());
        bytes[LINE + LINE_HANDLE..LINE + LINE_HANDLE + 8].copy_from_slice(&19_u64.to_le_bytes());
        bytes[LINE + cproduction_line::PRODUCTION_STATUS
            ..LINE + cproduction_line::PRODUCTION_STATUS + 8]
            .copy_from_slice(&(STATUS as u64).to_le_bytes());
        bytes[LINE + cbuilding_production_line::BUILDING
            ..LINE + cbuilding_production_line::BUILDING + 8]
            .copy_from_slice(&(TARGET as u64).to_le_bytes());
        for count in [0, 1, i32::MAX, i32::MIN] {
            bytes[LINE + cproduction_line::AMOUNT..LINE + cproduction_line::AMOUNT + 4]
                .copy_from_slice(&count.to_le_bytes());
            let memory = WithTarget(SliceMemory {
                base: 0,
                bytes: &bytes,
            });
            assert_eq!(
                count_reset(&memory, BASE, LINE),
                Ok((19, count, Some(captured_identity(false))))
            );
            assert_eq!(
                count_reset(&WithoutIdentity(&memory), BASE, LINE),
                Ok((19, count, None))
            );
            assert!(count_reset(&memory, BASE, 0).is_err());
            assert!(count_reset(&memory, BASE + 1, LINE).is_err());
        }
    }

    #[test]
    fn linked_completion_authenticates_the_native_removed_receiver_and_owner() {
        const STATUS: usize = 0x3000;
        struct LinkedMemory {
            line: [u8; cbuilding_production_line::BUILDING + 8],
            owner: [u8; engine_native::cproduction_status::CONSTRUCTION_LINES_COUNT + 4],
            slot: usize,
            date: bool,
        }
        impl Memory for LinkedMemory {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                let source: &[u8] = if (LINE..LINE + self.line.len()).contains(&address) {
                    return SliceMemory {
                        base: LINE,
                        bytes: &self.line,
                    }
                    .copy(address, out);
                } else if address == STATUS {
                    &self.owner
                } else if address == 0x4000 {
                    out.copy_from_slice(&self.slot.to_le_bytes());
                    return true;
                } else if address == BASE + engine_native::cgame_state::SINGLETON_RVA.0 {
                    out.copy_from_slice(&0x2000_usize.to_le_bytes());
                    return true;
                } else if address == 0x2000 + engine_native::cgame_state::CURRENT_HOUR {
                    if !self.date {
                        return false;
                    }
                    out.copy_from_slice(&120_i32.to_le_bytes());
                    return true;
                } else {
                    return target_regions(STATUS).copy(address, out);
                };
                let Some(source) = source.get(..out.len()) else {
                    return false;
                };
                out.copy_from_slice(source);
                true
            }
        }
        let mut memory = LinkedMemory {
            line: [0; cbuilding_production_line::BUILDING + 8],
            owner: [0; engine_native::cproduction_status::CONSTRUCTION_LINES_COUNT + 4],
            slot: LINE,
            date: true,
        };
        for (offset, word) in [
            (0, (BASE + LINE_VTABLES[0].0) as u64),
            (LINE_HANDLE, 7),
            (cproduction_line::PRODUCTION_STATUS, STATUS as u64),
            (cbuilding_production_line::BUILDING, TARGET as u64),
        ] {
            memory.line[offset..offset + 8].copy_from_slice(&word.to_le_bytes());
        }
        for (offset, word) in [
            (0, (BASE + engine_native::cproduction_status::VTABLE_RVA.0) as u64),
            (engine_native::cproduction_status::CONSTRUCTION_LINES, 0x4000),
        ] {
            memory.owner[offset..offset + 8].copy_from_slice(&word.to_le_bytes());
        }
        for offset in [
            engine_native::cproduction_status::CONSTRUCTION_LINES_COUNT,
            engine_native::cproduction_status::CONSTRUCTION_LINES_CAPACITY,
        ] {
            memory.owner[offset..offset + 4].copy_from_slice(&1_i32.to_le_bytes());
        }
        assert_eq!(
            linked_completion(&memory, BASE, STATUS, LINE).unwrap(),
            Completion {
                line: 7,
                identity: Some(captured_identity(false)),
                hour: 120,
                cancelled: false,
            }
        );
        memory.line[cbuilding_production_line::BUILDING..cbuilding_production_line::BUILDING + 8]
            .fill(0);
        assert_eq!(
            linked_completion(&memory, BASE, STATUS, LINE)
                .unwrap()
                .identity,
            None
        );
        memory.line[cbuilding_production_line::BUILDING..cbuilding_production_line::BUILDING + 8]
            .copy_from_slice(&(TARGET as u64).to_le_bytes());
        assert_eq!(
            linked_completion(&memory, BASE, STATUS + 1, LINE),
            Err("linked construction completion owner mismatch")
        );
        memory.line[cproduction_line::AMOUNT] = 1;
        assert_eq!(
            linked_completion(&memory, BASE, STATUS, LINE),
            Err("linked construction completion amount is not zero")
        );
        memory.line[cproduction_line::AMOUNT] = 0;
        memory.slot = LINE + 1;
        assert_eq!(
            linked_completion(&memory, BASE, STATUS, LINE),
            Err("linked construction completion roster receiver mismatch")
        );
        memory.slot = LINE;
        memory.date = false;
        assert_eq!(
            linked_completion(&memory, BASE, STATUS, LINE),
            Err("construction completion hour unreadable")
        );
    }

    #[test]
    fn completion_reads_identity_cancel_and_advanced_hour_without_spend_operands() {
        struct CompletionMemory {
            railway: bool,
            cancel: u8,
            short: bool,
        }
        impl Memory for CompletionMemory {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                if (LINE..LINE + crailway_production_line::COMPLETION_FLAG + 1).contains(&address) {
                    if self.short {
                        return false;
                    }
                    let mut bytes = [0_u8; crailway_production_line::COMPLETION_FLAG + 1];
                    bytes[..8].copy_from_slice(
                        &((BASE + LINE_VTABLES[usize::from(self.railway)].0) as u64).to_le_bytes(),
                    );
                    bytes[LINE_HANDLE..LINE_HANDLE + 8].copy_from_slice(&7_u64.to_le_bytes());
                    bytes[cproduction_line::PRODUCTION_STATUS
                        ..cproduction_line::PRODUCTION_STATUS + 8]
                        .copy_from_slice(&(STATUS as u64).to_le_bytes());
                    if !self.railway {
                        bytes[cbuilding_production_line::BUILDING
                            ..cbuilding_production_line::BUILDING + 8]
                            .copy_from_slice(&(TARGET as u64).to_le_bytes());
                    }
                    bytes[crailway_production_line::COMPLETION_FLAG] = self.cancel;
                    return SliceMemory {
                        base: LINE,
                        bytes: &bytes,
                    }
                    .copy(address, out);
                }
                if address == BASE + engine_native::cgame_state::SINGLETON_RVA.0 {
                    out.copy_from_slice(&0x2000_usize.to_le_bytes());
                    return true;
                }
                if address == 0x2000 + engine_native::cgame_state::CURRENT_HOUR {
                    out.copy_from_slice(&120_i32.to_le_bytes());
                    return true;
                }
                target_regions(STATUS).copy(address, out)
            }
        }
        for (railway, cancel) in [(false, 1), (true, 0), (true, 1)] {
            let memory = CompletionMemory {
                railway,
                cancel,
                short: false,
            };
            assert_eq!(
                completion(&memory, BASE, LINE, railway).unwrap(),
                Completion {
                    line: 7,
                    identity: Some(captured_identity(railway)),
                    hour: 120,
                    cancelled: railway && cancel != 0,
                }
            );
            assert_eq!(
                completion(&WithoutIdentity(&memory), BASE, LINE, railway).unwrap(),
                Completion {
                    line: 7,
                    identity: None,
                    hour: 120,
                    cancelled: railway && cancel != 0
                }
            );
        }
        let short = CompletionMemory {
            railway: false,
            cancel: 0,
            short: true,
        };
        assert_eq!(
            completion(&short, BASE, LINE, false),
            Err("construction completion line unreadable")
        );
    }
    struct Fixture {
        line: [u8; LINKED_LINE + 8],
        length: usize,
        instant: Option<u8>,
        invalid: Option<u64>,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            if address == LINE {
                return SliceMemory {
                    base: LINE,
                    bytes: &self.line[..self.length],
                }
                .copy(address, out);
            }
            if address == BASE + INSTANT_CONSTRUCTION.0 && out.len() == 1 {
                if let Some(flag) = self.instant {
                    out[0] = flag;
                    return true;
                }
                return false;
            }
            if address == BASE + INVALID_REFERENCE.0 && out.len() == 8 {
                if let Some(reference) = self.invalid {
                    out.copy_from_slice(&reference.to_le_bytes());
                    return true;
                }
                return false;
            }
            target_regions(STATUS).copy(address, out)
        }
    }
    fn fixture(railway: bool) -> Fixture {
        let mut fixture = Fixture {
            line: [0; LINKED_LINE + 8],
            length: LINKED_LINE + 8,
            instant: Some(0),
            invalid: Some(u64::MAX),
        };
        for (offset, value) in [
            (0, (BASE + LINE_VTABLES[usize::from(railway)].0) as u64),
            (LINE_HANDLE, 23),
            (cproduction_line::PRODUCTION_STATUS, STATUS as u64),
            (cbuilding_production_line::BUILDING, TARGET as u64),
            (cproduction_line::COST, 2_000_000),
            (cproduction_line::SPEED, 1_850_000),
            (CIC_BANK_DRAW, 50_000),
            (LINKED_LINE, u64::MAX),
        ] {
            fixture.line[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        fixture
    }
    #[test]
    fn entry_operands_copy_the_owner_and_target_with_the_spend() {
        let mut f = fixture(false);
        for levels in [-1_i32, 0, 2] {
            f.line[REPAIR_LEVELS..REPAIR_LEVELS + 4].copy_from_slice(&levels.to_le_bytes());
            let o = operands(&f, BASE, LINE, false).unwrap();
            assert_eq!(o.identity, Some(captured_identity(false)));
            let unread = operands(&WithoutIdentity(&f), BASE, LINE, false).unwrap();
            assert_eq!(unread.identity, None);
            assert_eq!(
                (unread.line, unread.bank, unread.cost, unread.speed),
                (o.line, o.bank, o.cost, o.speed)
            );
            assert_eq!(
                (o.line, o.bank, o.cost, o.speed),
                (23, 50_000, 2_000_000, 1_850_000)
            );
            assert_eq!(o.repair, levels > 0);
            assert!(o.partner.is_none() && !o.instant);
        }
        f.line[LINKED_LINE..LINKED_LINE + 8].copy_from_slice(&9_u64.to_le_bytes());
        f.instant = Some(1);
        let o = operands(&f, BASE, LINE, false).unwrap();
        assert_eq!(o.partner, Some(9));
        assert!(o.instant);
        let mut rail = fixture(true);
        rail.length = CIC_BANK_DRAW + 8;
        rail.invalid = None;
        rail.instant = None;
        let o = operands(&rail, BASE, LINE, true).unwrap();
        assert_eq!(o.identity, Some(captured_identity(true)));
        assert!(o.partner.is_none() && !o.repair);
    }

    #[test]
    fn native_empty_routes_have_no_endpoints_and_failed_target_reads_are_refused() {
        let mut memory = target_regions(STATUS);
        for (at, bytes) in &mut memory.0 {
            if *at == LINE + crailway_production_line::ROUTE {
                bytes[8..16].fill(0);
            }
        }
        let empty = identity(&memory, BASE, LINE, true, STATUS as u64, 0, None).unwrap();
        assert_eq!(
            (empty.country, empty.route_first, empty.route_last),
            (7, None, None)
        );
        for missing in [
            TARGET + cbuilding::TYPE_ID,
            TARGET_COUNTRY + engine_native::ccountry::PRODUCTION_STATUS,
            TARGET_STATE + engine_native::cstate::ID,
        ] {
            let mut memory = target_regions(STATUS);
            memory.0.retain(|(at, _)| *at != missing);
            assert!(
                identity(
                    &memory,
                    BASE,
                    LINE,
                    false,
                    STATUS as u64,
                    TARGET as u64,
                    None
                )
                .is_err()
            );
        }
    }
    #[test]
    fn short_or_unsupported_operands_are_refused_by_name() {
        let mut f = fixture(false);
        f.length -= 1;
        assert_eq!(
            operands(&f, BASE, LINE, false),
            Err("construction output line unreadable")
        );
        f.length += 1;
        f.line[0] ^= 1;
        assert_eq!(
            operands(&f, BASE, LINE, false),
            Err("construction output line vtable unsupported")
        );
        f.line[0] ^= 1;
        f.line[LINE_HANDLE..LINE_HANDLE + 8].fill(0);
        assert_eq!(
            operands(&f, BASE, LINE, false),
            Err("construction output line handle is null")
        );
        let mut f = fixture(false);
        f.instant = None;
        assert_eq!(
            operands(&f, BASE, LINE, false),
            Err("construction output instant flag unreadable")
        );
        let mut f = fixture(false);
        f.invalid = None;
        assert_eq!(
            operands(&f, BASE, LINE, false),
            Err("construction output invalid reference unreadable")
        );
    }

    struct Regions(Vec<(usize, Vec<u8>)>);
    impl Memory for Regions {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.0.iter().any(|(start, bytes)| {
                let Some(source) = address
                    .checked_sub(*start)
                    .and_then(|offset| bytes.get(offset..offset.checked_add(out.len())?))
                else {
                    return false;
                };
                out.copy_from_slice(source);
                true
            })
        }
    }
    const STATUS: usize = 0x2000;
    const COUNTRY: usize = 0x3000;
    const STATE: usize = 0x8000;
    const LINKS: usize = 0x9000;
    fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
        bytes[offset..offset + value.len()].copy_from_slice(value);
    }
    fn instant_world(flag: u8, playing: u32, observed: u32) -> Regions {
        let mut line = vec![0; 0x30];
        put(
            &mut line,
            0,
            &((BASE + crailway_production_line::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(&mut line, LINE_HANDLE, &0x2a_0000_1269_u64.to_le_bytes());
        put(
            &mut line,
            cproduction_line::PRODUCTION_STATUS,
            &(STATUS as u64).to_le_bytes(),
        );
        let mut status = vec![0; engine_native::cproduction_status::COUNTRY + 8];
        put(
            &mut status,
            0,
            &((BASE + engine_native::cproduction_status::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut status,
            engine_native::cproduction_status::COUNTRY,
            &(COUNTRY as u64).to_le_bytes(),
        );
        let mut country = vec![0; engine_native::ccountry::PRODUCTION_STATUS + 8];
        put(
            &mut country,
            0,
            &((BASE + engine_native::ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(&mut country, engine_native::ccountry::ID, &5_u32.to_le_bytes());
        put(
            &mut country,
            engine_native::ccountry::PRODUCTION_STATUS,
            &(STATUS as u64).to_le_bytes(),
        );
        let mut state = vec![0; cgame_state::LOCAL_PLAYER_COUNTRY + 8];
        for (offset, value) in [
            (cgame_state::COUNTRY_LINKS, (LINKS as u64).to_le_bytes()),
            (
                cgame_state::COUNTRY_LINKS + 8,
                (8_u64 | 8 << 32).to_le_bytes(),
            ),
            (
                cgame_state::LOCAL_PLAYER_COUNTRY,
                (u64::from(playing) | u64::from(observed) << 32).to_le_bytes(),
            ),
        ] {
            put(&mut state, offset, &value);
        }
        let links: Vec<u8> = [0_u32, 1, 2, 3, 4, 5, 6, 5]
            .iter()
            .flat_map(|link| link.to_le_bytes())
            .collect();
        Regions(vec![
            (BASE + INSTANT_CONSTRUCTION.0, vec![flag]),
            (
                BASE + cgame_state::SINGLETON_RVA.0,
                (STATE as u64).to_le_bytes().to_vec(),
            ),
            (LINE, line),
            (STATUS, status),
            (COUNTRY, country),
            (STATE, state),
            (LINKS, links),
        ])
    }
    #[test]
    fn railway_instant_follows_the_native_flag_and_viewing_country_gate() {
        let clear = Regions(vec![(BASE + INSTANT_CONSTRUCTION.0, vec![0])]);
        assert_eq!(railway_instant(&clear, BASE, LINE), Ok(false));
        for (playing, observed, instant) in [
            (5, 0, true),
            (7, 0, true),
            (6, 5, false),
            (0, 5, true),
            (u32::MAX, 7, true),
            (0, 6, false),
            (0, 0, false),
        ] {
            assert_eq!(
                railway_instant(&instant_world(1, playing, observed), BASE, LINE),
                Ok(instant),
                "playing {playing}, observed {observed}"
            );
            assert_eq!(
                railway_instant(&instant_world(0, playing, observed), BASE, LINE),
                Ok(false)
            );
        }
        let mut world = instant_world(1, 7, 0);
        world.0.retain(|(start, _)| *start != LINKS);
        assert_eq!(
            railway_instant(&world, BASE, LINE),
            Err("construction output instant country link unreadable")
        );
        let unreadable = Regions(vec![]);
        assert_eq!(
            railway_instant(&unreadable, BASE, LINE),
            Err("construction output instant flag unreadable")
        );
    }
}
