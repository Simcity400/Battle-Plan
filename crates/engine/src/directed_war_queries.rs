//! Exact directed-war query demand, without retaining predicate values.
//!
//! Interior observations precede the original status+2E8 load. They retain
//! only authenticated receiver, target and immutable matrix-cell identities.
//! Fresh predicate acquisition belongs to the admitted hourly diplomacy pass;
//! no CRelation pointer or retired byte crosses a callback or world occurrence.

use crate::{Memory, ccountry, cdiplomacy_status, cpdx_array::Array, crelation_status, read};

/// Existing directed packets represent positive receiver countries only.
pub const UNSUPPORTED_RECEIVER: &str = "directed war query nonpositive receiver unsupported";

pub struct Site {
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// War queries reach this load after self/alias, positive-receiver and
/// non-null-status gates: RSI receiver, RDI raw/canonical target pair, RAX status.
/// Friendly queries use RDI receiver, RSI target pair and RBX status. Both
/// copied seven-byte spans are loads with no interior branch or return address,
/// stack change or RIP operand. They share their original handler-free unwind
/// state; observe before resuming the relocated load.
pub const SITES: [Site; 2] = [
    Site {
        rva: 0x006f_3a52,
        sentinel: [
            0x48, 0x8b, 0x80, 0xe8, 2, 0, 0, 0x48, 0x85, 0xc0, 0x74, 0x18, 0x80, 0x78, 0x49, 0,
        ],
    },
    Site {
        rva: 0x006f_3b28,
        sentinel: [
            0x48, 0x8b, 0x83, 0xe8, 2, 0, 0, 0x48, 0x85, 0xc0, 0x74, 0x18, 0x80, 0x78, 0x49, 0,
        ],
    },
];

#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub receiver: usize,
    pub target: usize,
    pub status: usize,
}

/// Pointers are callback/world-scoped demand keys, never replay IDs. Every
/// diplomacy object eagerly owns its status matrix, with fixed target and
/// owner backlinks. Destruction retires the matrix with its world. Collector
/// invalidates on reset, destruction and construction and reauthenticates at capture.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Demand {
    pub receiver: usize,
    pub receiver_raw: u32,
    pub receiver_instance: u32,
    pub diplomacy: usize,
    pub status: usize,
    pub target_raw: u32,
    pub target_slot: u32,
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Option<usize> {
    object
        .checked_add(offset)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|value| usize::try_from(value).ok())
        .filter(|&value| value != 0)
}

/// Read identities only. Do not observe a between-tick value as an hourly fact.
/// A positive raw target linked to slot zero authenticates the same reserved
/// object as raw zero; its original raw identity remains in the demand.
pub fn read_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    frame: Frame,
) -> Result<Demand, &'static str> {
    let raw = frame
        .receiver
        .checked_add(ccountry::ID)
        .filter(|_| frame.receiver != 0)
        .and_then(|at| read::<u32>(memory, at))
        .ok_or("directed war query receiver identity unreadable")?;
    if raw as i32 <= 0 {
        return Err(UNSUPPORTED_RECEIVER);
    }
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let receiver = countries.of_pointer(frame.receiver)?;
    if receiver.id as i32 <= 0 || receiver.canonical_slot == 0 {
        return Err(UNSUPPORTED_RECEIVER);
    }
    let receiver_instance = receiver
        .pointer
        .checked_add(ccountry::ID_INSTANCE)
        .and_then(|at| read::<u32>(memory, at))
        .ok_or("directed war query receiver instance unreadable")?;
    let diplomacy = pointer(memory, receiver.pointer, ccountry::DIPLOMACY_STATUS)
        .ok_or("directed war query diplomacy pointer unreadable or null")?;
    let expected = base
        .checked_add(cdiplomacy_status::VTABLE_RVA)
        .ok_or("directed war query module address overflow")? as u64;
    if read::<u64>(memory, diplomacy) != Some(expected)
        || pointer(memory, diplomacy, cdiplomacy_status::COUNTRY) != Some(receiver.pointer)
    {
        return Err("directed war query diplomacy identity refused");
    }
    let mut pair = [0; 8];
    if frame.target == 0 || !memory.copy(frame.target, &mut pair) {
        return Err("directed war query target pair unreadable");
    }
    let target_raw = u32::from_le_bytes(pair[..4].try_into().unwrap());
    let signed_slot = i32::from_le_bytes(pair[4..].try_into().unwrap());
    let target_slot = u32::try_from(signed_slot)
        .map_err(|_| "directed war query target canonical slot is negative")?;
    let target = resolve_target(&countries, target_raw)?;
    if target.canonical_slot != target_slot {
        return Err("directed war query target canonical slot differs from resolver");
    }
    let matrix = diplomacy
        .checked_add(cdiplomacy_status::RELATIONS)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("directed war query matrix header refused")?;
    authenticate_status(
        memory,
        base,
        diplomacy,
        matrix,
        frame.status,
        target,
        |raw| resolve_target(&countries, raw),
    )?;
    Ok(Demand {
        receiver: receiver.pointer,
        receiver_raw: receiver.id,
        receiver_instance,
        diplomacy,
        status: frame.status,
        target_raw,
        target_slot,
    })
}

pub(crate) fn resolve_target<M: Memory>(
    countries: &ccountry::Reader<'_, M>,
    raw: u32,
) -> Result<ccountry::Country, &'static str> {
    if raw == 0 {
        countries.reserved()
    } else {
        match countries.link(raw)? {
            None => countries.reserved(),
            Some(_) => countries.resolve(raw),
        }
    }
}

/// Authenticate the exact reached cell, without reading status+2E8 or CRelation.
/// The passed canonical target selects the exact matrix cell.
#[allow(clippy::too_many_arguments)]
pub(crate) fn authenticate_status(
    memory: &impl Memory,
    base: usize,
    diplomacy: usize,
    matrix: Array,
    status: usize,
    target: ccountry::Country,
    mut resolve: impl FnMut(u32) -> Result<ccountry::Country, &'static str>,
) -> Result<(), &'static str> {
    let selected = matrix
        .element(target.canonical_slot, 8)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|value| usize::try_from(value).ok())
        .filter(|&value| value != 0)
        .ok_or("directed war query matrix target missing or unreadable")?;
    if selected != status {
        return Err("directed war query status differs from selected matrix cell");
    }
    let expected = base
        .checked_add(crelation_status::VTABLE_RVA)
        .ok_or("directed war query module address overflow")? as u64;
    if read::<u64>(memory, status) != Some(expected)
        || pointer(memory, status, crelation_status::DIPLOMACY_STATUS) != Some(diplomacy)
    {
        return Err("directed war query status identity refused");
    }
    let raw = status
        .checked_add(crelation_status::TARGET_COUNTRY)
        .and_then(|at| read::<u32>(memory, at))
        .ok_or("directed war query status target unreadable")?;
    let actual = resolve(raw)?;
    if actual.pointer != target.pointer || actual.canonical_slot != target.canonical_slot {
        return Err("directed war query status target differs from demand");
    }
    Ok(())
}
