//! Construction progress observation boundaries. The game keeps no record of the
//! civilian-factory output a construction line consumed; collector `construction_ic`
//! observes these calls without invoking them.
//!
//! Construction advances in the daily simulation-thread production pass
//! ([`crate::line_output_sites`]). Every general construction occurrence is advanced
//! and then independently checked for completion; completed lines are removed.
//!
//! A linked building line whose partner is primary returns early. Every other
//! building path consumes [`BUILDING_SPEED`] at [`BUILDING_SPEND_RETURN`], adding
//! the line's speed to produced progress when no levels need repair, or to repair
//! otherwise. [`INSTANT_CONSTRUCTION`] raises the amount to at least the cost;
//! otherwise [`CIC_BANK_DRAW`] is withdrawn from the country's bank after spending.
//! The repair path's additional speed query returns at a different boundary.
//!
//! Railway advancement reads stored speed and passes it to [`RAILWAY_PROGRESS`]
//! for the first unbuilt route segment. Excess progress can carry to the next
//! segment through a second call, then the normal loop stops and draws the bank.
//! Speed below one reaches neither progress call. Instant construction applies
//! only to the player's country and passes exactly 10^10 per segment; other
//! countries spend normally and draw the bank even while the flag is set.
//!
//! The CIC bank receives effect and international-market contributions, rather
//! than daily factory production. Speed updates reserve the bank draw and add it
//! to stored speed. Railway-gun repair also spends civilian output but has no
//! construction table.

use crate::{
    Memory, cbuilding_production_line, cgeneral_production_line, cproduction_line,
    crailway_production_line, read,
};

/// Storage demand for one daily construction pass, never captured data.
/// Every construction occurrence is visited. Building advancement has one spend,
/// one possible linked removal, then an independent completion decision. Railway
/// advancement has at most one call per route edge: instant construction advances
/// every edge; normal advancement carries only when another edge exists, then stops.
/// Railway completion is checked independently.
pub fn events_per_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let countries = crate::ccountry::Reader::new(memory, base, game_state)?;
    let mut events = 0_usize;
    let expected = |rva| {
        base.checked_add(rva)
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
        let status = crate::cproduction_status::manager(memory, base, country)?;
        let lines = status
            .checked_add(crate::cproduction_status::CONSTRUCTION_LINES)
            .and_then(|address| crate::cpdx_array::Array::read(memory, address, 8))
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

/// A detoured native function.
pub struct Site {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// Building speed query: RCX is the line, RDX is output storage, and the return
/// value is that storage. It supplies this line's speed or a linked non-primary
/// line's partner's speed. The relocated prefix saves RBX to the stack; the
/// following RIP-relative load remains at its original location.
pub const BUILDING_SPEED: Site = Site {
    name: "building speed",
    rva: 0x00f6_e060,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0x05, 0xbc, 0xf4, 0x3c,
        0x02,
    ],
};
/// Return RVA of the [`BUILDING_SPEED`] call whose value building advancement spends.
pub const BUILDING_SPEND_RETURN: usize = 0x00f6_a877;

/// Building completion query: RCX is the line and AL is the result.
/// Amount and repair count must both be zero.
pub const BUILDING_COMPLETE: Site = Site {
    name: "building completion",
    rva: 0x00f6_e160,
    sentinel: [
        0x48, 0x83, 0xec, 0x28, 0x83, 0x79, 0x48, 0x00, 0x75, 0x14, 0x48, 0x8b, 0x01, 0xff, 0x90,
        0xa8,
    ],
};

/// Completion clears amount and repair count before ownership transfer rescales
/// shared levels. Observe the count before the original. The copied prefix saves
/// RBX and reserves stack space before the indirect call.
pub const BUILDING_RESET: Site = Site {
    name: "building count reset",
    rva: 0x00f6_a510,
    sentinel: [
        0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x01, 0x33, 0xd2, 0x48, 0x8b, 0xd9, 0xff,
        0x90,
    ],
};

pub fn count_reset(
    memory: &impl Memory,
    base: usize,
    line: usize,
) -> Result<(u64, i32), &'static str> {
    if line == 0 || read::<u64>(memory, line) != base.checked_add(LINE_VTABLES[0]).map(|p| p as u64)
    {
        return Err("construction count reset line vtable unreadable or unsupported");
    }
    let handle = line
        .checked_add(LINE_HANDLE)
        .and_then(|p| read::<u64>(memory, p))
        .ok_or("construction count reset handle unreadable")?;
    let count = line
        .checked_add(cproduction_line::AMOUNT)
        .and_then(|p| read::<i32>(memory, p))
        .ok_or("construction count reset amount unreadable")?;
    Ok((handle, count))
}

/// Railway completion query: RCX is the line and AL is the result.
/// Cancellation or every adjacent route edge reaching the target level completes it.
pub const RAILWAY_COMPLETE: Site = Site {
    name: "railway completion",
    rva: 0x01a0_3a60,
    sentinel: [
        0x40, 0x55, 0x48, 0x83, 0xec, 0x50, 0x80, 0x3d, 0xe5, 0xe0, 0xbd, 0x01, 0x00, 0x40, 0xb5,
        0x01,
    ],
};

/// The daily production pass checks completion after advancing the line and
/// removes and deletes it on a true result. Other getter calls are not
/// queue-completion events.
pub const COMPLETE_RETURN: usize = 0x00e6_732e;

/// General-line removal: RCX is the status and RDX is the line.
/// It removes the roster index and destroys the line.
pub const REMOVE_GENERAL_LINE: Site = Site {
    name: "linked building completion removal",
    rva: 0x00e6_efa0,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x30,
        0x48,
    ],
};
/// Primary advancement decrements its partner, tests for zero amount, and
/// deletes it before the daily completion check. Command and hourly removal
/// callers are not completion events.
pub const LINKED_COMPLETE_RETURN: usize = 0x00f6_ad8c;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Completion {
    pub line: u64,
    pub hour: i32,
    pub cancelled: bool,
}

/// The receiver remains alive until its native caller deletes it. Read only its
/// identity, railway cancel byte and native date, never spend operands.
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
    if base.checked_add(LINE_VTABLES[usize::from(railway)]) != Some(word(0) as usize) {
        return Err("construction completion line vtable unsupported");
    }
    let handle = word(LINE_HANDLE);
    if handle == 0 {
        return Err("construction completion line handle is null");
    }
    let state = base
        .checked_add(crate::cgame_state::SINGLETON_RVA)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|p| *p != 0)
        .ok_or("construction completion game state unreadable")?;
    let hour = state
        .checked_add(crate::cgame_state::CURRENT_HOUR)
        .and_then(|p| read::<i32>(memory, p))
        .ok_or("construction completion hour unreadable")?;
    // The calendar advances before daily updates: this is the advanced hour
    // and the next sample's boundary.
    Ok(Completion {
        line: handle,
        hour,
        // Cancellation byte, serialized as `cancel`.
        cancelled: railway && bytes[crailway_production_line::COMPLETION_FLAG] != 0,
    })
}

/// Called before general-line removal destroys the completed linked
/// receiver. Its owner and priority select the roster slot that will be
/// removed; authenticate that slot before attributing the event to its CID.
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
    let mut owner = [0; crate::cproduction_status::CONSTRUCTION_LINES_COUNT + 4];
    if !memory.copy(status, &mut owner) {
        return Err("linked construction completion owner unreadable");
    }
    let owner_word = |offset| u64::from_le_bytes(owner[offset..offset + 8].try_into().unwrap());
    if base.checked_add(crate::cproduction_status::VTABLE_RVA) != Some(owner_word(0) as usize) {
        return Err("linked construction completion owner vtable unsupported");
    }
    let owner_int = |offset| i32::from_le_bytes(owner[offset..offset + 4].try_into().unwrap());
    let count = owner_int(crate::cproduction_status::CONSTRUCTION_LINES_COUNT);
    let capacity = owner_int(crate::cproduction_status::CONSTRUCTION_LINES_CAPACITY);
    let index = int(cproduction_line::PRIORITY);
    if index < 0 || index >= count || count > capacity {
        return Err("linked construction completion roster index invalid");
    }
    let slot = usize::try_from(owner_word(crate::cproduction_status::CONSTRUCTION_LINES))
        .ok()
        .filter(|p| *p != 0)
        .and_then(|p| {
            (index as usize)
                .checked_mul(crate::cproduction_status::CONSTRUCTION_POINTER_STRIDE)
                .and_then(|offset| p.checked_add(offset))
        })
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok());
    if slot != Some(line) {
        return Err("linked construction completion roster receiver mismatch");
    }
    Ok(event)
}

/// Railway advancement entry: RCX is the line.
/// The relocated prefix comprises four register pushes occupying five bytes.
pub const RAILWAY_ADVANCE: Site = Site {
    name: "railway advance",
    rva: 0x01a0_2660,
    sentinel: [
        0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8b, 0xec, 0x48,
        0x83,
    ],
};

/// Railway progress query: RCX is the railway manager, EDX is the source province, R8D is the
/// destination province, R9 is the signed i64 amount, and the country is in its stack argument
/// slot. The boolean result reports segment completion. The relocated prefix saves RBX to the
/// stack.
pub const RAILWAY_PROGRESS: Site = Site {
    name: "railway progress",
    rva: 0x00e9_3090,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
        0x41,
    ],
};

/// Instant-construction console flag (u8). Progress under it consumes the
/// cost and is not factory output.
pub const INSTANT_CONSTRUCTION: usize = 0x0332_f628;
/// Railway instant progress amount passed in R9, gated by the player's country.
pub const RAILWAY_INSTANT_AMOUNT: i64 = 10_000_000_000;
/// `creference_registry::invalid_reference` (u64): the empty reference, assigned
/// at run time (read it live, never a sampled constant).
pub const INVALID_REFERENCE: usize = 0x0333_d528;

/// The line's own CID (u32 type, u32 id): the construction tables' `id`.
pub const LINE_HANDLE: usize = cproduction_line::REFERENCE_TYPE;
/// CIC bank draw included in the line's speed (i64 fixed5), reserved during
/// speed updates and withdrawn after the spend.
pub const CIC_BANK_DRAW: usize = cgeneral_production_line::CIC_BANK_DRAW;
/// A building line's levels still to repair (i32): above zero its advance repairs.
pub const REPAIR_LEVELS: usize = cbuilding_production_line::REPAIR_LEVELS_REMAINING;
/// Partner's full reference handle: low u32 type and high u32 ID, matching
/// LINE_HANDLE. Speed updates resolve it and include the partner's factories.
/// The linked-line reference is valid for lines built together across countries.
pub const LINKED_LINE: usize = cbuilding_production_line::LINKED_LINE;

/// The vtables of the construction lines whose output is observed, building then
/// railway.
pub const LINE_VTABLES: [usize; 2] = [
    cbuilding_production_line::VTABLE_RVA,
    crailway_production_line::VTABLE_RVA,
];

/// Only the detour's live line and the two named globals are copied. No owner,
/// building, route or definition pointer is followed. The native advance keeps
/// its receiver alive through both observation sites.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operands {
    pub line: u64,
    pub bank: i64,
    pub cost: i64,
    /// At BUILDING_SPEND_RETURN the getter returns this object's own value.
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
    if base.checked_add(LINE_VTABLES[usize::from(railway)]) != Some(word(0) as usize) {
        return Err("construction output line vtable unsupported");
    }
    let handle = word(LINE_HANDLE);
    if handle == 0 {
        return Err("construction output line handle is null");
    }
    let instant = !railway
        && base
            .checked_add(INSTANT_CONSTRUCTION)
            .and_then(|p| read::<u8>(memory, p))
            .ok_or("construction output instant flag unreadable")?
            != 0;
    let invalid = if railway {
        0
    } else {
        base.checked_add(INVALID_REFERENCE)
            .and_then(|p| read::<u64>(memory, p))
            .ok_or("construction output invalid reference unreadable")?
    };
    Ok(Operands {
        line: handle,
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
    use crate::memory::SliceMemory;

    const BASE: usize = 0x140000000;
    const LINE: usize = 0x1000;

    #[test]
    fn count_reset_observes_the_native_signed_count_before_it_is_cleared() {
        let mut bytes = vec![0; LINE + cproduction_line::AMOUNT + 4];
        bytes[LINE..LINE + 8].copy_from_slice(&((BASE + LINE_VTABLES[0]) as u64).to_le_bytes());
        bytes[LINE + LINE_HANDLE..LINE + LINE_HANDLE + 8].copy_from_slice(&19_u64.to_le_bytes());
        for count in [0, 1, i32::MAX, i32::MIN] {
            bytes[LINE + cproduction_line::AMOUNT..LINE + cproduction_line::AMOUNT + 4]
                .copy_from_slice(&count.to_le_bytes());
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            assert_eq!(count_reset(&memory, BASE, LINE), Ok((19, count)));
            assert!(count_reset(&memory, BASE, 0).is_err());
            assert!(count_reset(&memory, BASE + 1, LINE).is_err());
        }
    }

    #[test]
    fn linked_completion_authenticates_the_native_removed_receiver_and_owner() {
        const STATUS: usize = 0x3000;
        struct LinkedMemory {
            line: [u8; cproduction_line::AMOUNT + 4],
            owner: [u8; crate::cproduction_status::CONSTRUCTION_LINES_COUNT + 4],
            slot: usize,
            date: bool,
        }
        impl Memory for LinkedMemory {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                let source: &[u8] = if address == LINE {
                    &self.line
                } else if address == STATUS {
                    &self.owner
                } else if address == 0x4000 {
                    out.copy_from_slice(&self.slot.to_le_bytes());
                    return true;
                } else if address == BASE + crate::cgame_state::SINGLETON_RVA {
                    out.copy_from_slice(&0x2000_usize.to_le_bytes());
                    return true;
                } else if address == 0x2000 + crate::cgame_state::CURRENT_HOUR {
                    if !self.date {
                        return false;
                    }
                    out.copy_from_slice(&120_i32.to_le_bytes());
                    return true;
                } else {
                    panic!("unexpected linked completion address: {address:#x}")
                };
                let Some(source) = source.get(..out.len()) else {
                    return false;
                };
                out.copy_from_slice(source);
                true
            }
        }
        let mut memory = LinkedMemory {
            line: [0; cproduction_line::AMOUNT + 4],
            owner: [0; crate::cproduction_status::CONSTRUCTION_LINES_COUNT + 4],
            slot: LINE,
            date: true,
        };
        for (offset, word) in [
            (0, (BASE + LINE_VTABLES[0]) as u64),
            (LINE_HANDLE, 7),
            (cproduction_line::PRODUCTION_STATUS, STATUS as u64),
        ] {
            memory.line[offset..offset + 8].copy_from_slice(&word.to_le_bytes());
        }
        for (offset, word) in [
            (0, (BASE + crate::cproduction_status::VTABLE_RVA) as u64),
            (crate::cproduction_status::CONSTRUCTION_LINES, 0x4000),
        ] {
            memory.owner[offset..offset + 8].copy_from_slice(&word.to_le_bytes());
        }
        for offset in [
            crate::cproduction_status::CONSTRUCTION_LINES_COUNT,
            crate::cproduction_status::CONSTRUCTION_LINES_CAPACITY,
        ] {
            memory.owner[offset..offset + 4].copy_from_slice(&1_i32.to_le_bytes());
        }
        assert_eq!(
            linked_completion(&memory, BASE, STATUS, LINE).unwrap(),
            Completion {
                line: 7,
                hour: 120,
                cancelled: false,
            }
        );
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
                if address == LINE {
                    if self.short {
                        return false;
                    }
                    let mut bytes = [0_u8; 0x79];
                    bytes[..8].copy_from_slice(
                        &((BASE + LINE_VTABLES[usize::from(self.railway)]) as u64).to_le_bytes(),
                    );
                    bytes[LINE_HANDLE..LINE_HANDLE + 8].copy_from_slice(&7_u64.to_le_bytes());
                    bytes[0x78] = self.cancel;
                    out.copy_from_slice(&bytes[..out.len()]);
                    return true;
                }
                if address == BASE + crate::cgame_state::SINGLETON_RVA {
                    out.copy_from_slice(&0x2000_usize.to_le_bytes());
                    return true;
                }
                if address == 0x2000 + crate::cgame_state::CURRENT_HOUR {
                    out.copy_from_slice(&120_i32.to_le_bytes());
                    return true;
                }
                panic!("completion followed a spend operand: {address:#x}");
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
                    hour: 120,
                    cancelled: railway && cancel != 0,
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
            if address == BASE + INSTANT_CONSTRUCTION && out.len() == 1 {
                if let Some(flag) = self.instant {
                    out[0] = flag;
                    return true;
                }
                return false;
            }
            if address == BASE + INVALID_REFERENCE && out.len() == 8 {
                if let Some(reference) = self.invalid {
                    out.copy_from_slice(&reference.to_le_bytes());
                    return true;
                }
                return false;
            }
            panic!("hook followed another address: {address:#x}");
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
            (0, (BASE + LINE_VTABLES[usize::from(railway)]) as u64),
            (LINE_HANDLE, 23),
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
    fn entry_operands_only_read_the_line_and_named_globals() {
        let mut f = fixture(false);
        for levels in [-1_i32, 0, 2] {
            f.line[REPAIR_LEVELS..REPAIR_LEVELS + 4].copy_from_slice(&levels.to_le_bytes());
            let o = operands(&f, BASE, LINE, false).unwrap();
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
        assert!(o.partner.is_none() && !o.repair);
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
}
