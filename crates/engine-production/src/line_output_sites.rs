//! Production output observation sites. The game keeps no count of the units
//! a production line has completed; observing CREDIT and DELIVERY supplies it
//! to collector `line_output`. Rocket spending has a separate entry reader.
//!
//! Production runs in a serial daily country pass after parallel work finishes.
//! Each equipment line stays alive through refresh and production, then may be
//! removed when its remaining amount is zero.
//!
//! Military, naval, railway-gun and refit lines credit the tick's signed count.
//! Delivery precedes credit when equipment is delivered, with the amount scaled
//! by [`UNIT_SCALE`]. A naval line blocked by manpower still credits its count
//! and sets progress to cost. Military and railway-gun ticks without completion
//! can credit zero or a negative count; a blocked railway gun credits zero.
//! A zero cost supplies 42949 units. Conversion can deliver and credit zero;
//! refit credits one exactly when it delivers a ship.
//!
//! CREDIT receives the signed progress input after resource penalties, including
//! ticks without delivery. Blocked naval deployment discards that progress;
//! a blocked railway gun keeps it. Instant construction replaces the input
//! with the signed maximum of speed and cost. Instant training applies the same
//! replacement to the viewing country's naval and railway lines. These are
//! instant progress, separate from factory output. Country-reference comparison
//! selects the viewing country; commands cannot interleave with the serial pass.
//!
//! Rocket lines deliver without CREDIT. Their signed stored progress input is
//! replaced by the signed maximum of speed and variant cost for instant
//! construction. A null variant skips spending. The entry reader does not refresh
//! factory output or require a line handle or registration.

use crate::{
    Memory, ccountry, cequipment_production_line, cequipment_variant, cgame_state,
    cmilitary_production_line, cnaval_production_line, cpdx_array::Array, cproduction_line,
    cproduction_status, crailway_gun_production_line, cship_refit_production_line, read,
};
use engine_sites::Rva;

pub struct HookSite {
    pub name: &'static str,
    pub rva: Rva,
    /// The return address (RVA) of each equipment-production call, by
    /// [`PRODUCERS`] index: the same index pairs a delivery with its credit.
    pub callers: [Rva; 4],
}

/// An entry observation site that needs no caller classification.
pub struct EntrySite {
    pub rva: Rva,
}

/// Equipment-production implementations, in the order of [`HookSite::callers`].
pub const PRODUCERS: [&str; 4] = ["military", "naval", "railway gun", "ship refit"];

/// Credit entry: RCX is the line, RDX the signed progress input and R8D the
/// completed count. A set, resolving organisation handle selects the variant's
/// history increment. The call itself does not write the line, but crediting
/// the organisation can unlock its automatic design upgrade, which reassigns
/// factories and refreshes the producer's lines before it returns: read the
/// line at entry. Its entry instruction spills RBX to the stack.
pub const CREDIT: HookSite = HookSite {
    name: "credit",
    rva: Rva(0x019b_ece0),
    callers: [
        Rva(0x00ff_2ef7),
        Rva(0x00ff_3980),
        Rva(0x00ff_478c),
        Rva(0x00ff_62f5),
    ],
};

/// Delivery entry: RCX is the country's logistics status, RDX the variant
/// pointer and R8 the signed i64 amount. Equipment production calls it on
/// delivery with units times [`UNIT_SCALE`]. Its entry instruction spills R8
/// to the stack.
pub const DELIVERY: HookSite = HookSite {
    name: "delivery",
    rva: Rva(0x0140_ab50),
    callers: [
        Rva(0x00ff_2e94),
        Rva(0x00ff_38ec),
        Rva(0x00ff_440d),
        Rva(0x00ff_62d0),
    ],
};

/// Rocket production entry; RCX is the line. Its six-byte entry prefix has
/// no relative operands. The daily caller ignores RAX, which observers preserve.
pub const ROCKET_ADVANCE: EntrySite = EntrySite {
    rva: Rva(0x00ff_5980),
};

/// DELIVERY's rocket-production caller.
pub const ROCKET_DELIVERY_RETURN: Rva = Rva(0x00ff_5a7a);
/// Rocket primary vtable.
pub const ROCKET_VTABLE: Rva = Rva(0x029f_8a98);
/// Rocket's owning production status.
pub const ROCKET_STATUS: usize = 0x38;
/// Rocket's bound variant pointer, passed to DELIVERY.
pub const ROCKET_VARIANT: usize = 0x40;
/// Stored signed progress input added during rocket production.
pub const ROCKET_SPEED: usize = 0x60;

/// [`DELIVERY`]'s fixed-point amount scale. Callers widen the signed unit
/// count to i64 before multiplying by this value; refit delivers one ship.
pub const UNIT_SCALE: i64 = 100_000;

/// Production status's separate rocket pointer roster, with a signed count.
/// This header supplies sizing only; it acquires no rocket census or payload.
pub const ROCKET_LINES: usize = 0x88;

/// One daily pass can append one equipment tick and at most two events per
/// rocket: spending and delivery. Header hints allocate off-thread without
/// visiting a producer pointer, refreshing it or calling a game function.
pub fn output_events_per_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut events = 0usize;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let status = cproduction_status::manager(memory, base, country)?;
        let mut equipment = [0; 16];
        let mut rockets = [0; 16];
        let addresses = [cproduction_status::LINES, ROCKET_LINES].map(|offset| {
            status
                .checked_add(offset)
                .ok_or("production output header address overflow")
        });
        let [equipment_at, rocket_at] = addresses;
        if memory.copy_scattered(
            [equipment_at?, rocket_at?],
            &mut [&mut equipment, &mut rockets],
        ) != 2
        {
            return Err("production output sizing headers unreadable");
        }
        let equipment =
            Array::from_header(equipment, 8).ok_or("production output equipment header invalid")?;
        let rockets =
            Array::from_header(rockets, 8).ok_or("production output Rocket header invalid")?;
        let rocket_events = (rockets.count as usize)
            .checked_mul(2)
            .ok_or("production output Rocket event demand overflow")?;
        events = events
            .checked_add(equipment.count as usize)
            .and_then(|events| events.checked_add(rocket_events))
            .ok_or("production output event demand overflow")?;
    }
    Ok(events)
}
/// The line's own CID (u32 type, u32 id), `ProductionLines.handle`.
pub const LINE_HANDLE: usize = cproduction_line::REFERENCE_TYPE;
/// The produced variant's pointer, passed to both observation calls.
pub const LINE_VARIANT: usize = cequipment_production_line::EQUIPMENT_VARIANT;
/// The organisation CID [`CREDIT`] resolves, zero for none.
pub const LINE_ORGANISATION: usize = cequipment_production_line::INDUSTRIAL_ORGANISATION_CID;
/// The variant's CID, the key of the organisation's history slot.
pub const VARIANT_HANDLE: usize = cequipment_variant::TYPE;
/// The line's conversion control, which only military production reads.
pub const LINE_CONVERSION: usize = cequipment_production_line::CONVERSION;

/// Whether a military production call converts stockpiled equipment into
/// `line`'s design: its conversion control is set and the design is a
/// conversion target ([`crate::production_modifiers::conversion_target`]).
/// The call takes its conversion branch on exactly these operands, so read
/// them at CREDIT entry: CREDIT can refresh the producer's lines before it
/// returns (an organisation's automatic design upgrade reassigns factories),
/// which may clear the control or replace the design. Such a call's whole
/// input is conversion progress, including ticks that complete nothing; it
/// does not prove a source unit was removed. Naval, railway-gun and refit
/// production have no conversion branch. No game function is called.
pub fn converting(memory: &impl Memory, line: usize) -> Result<bool, &'static str> {
    let control = line
        .checked_add(LINE_CONVERSION)
        .and_then(|address| read::<u8>(memory, address))
        .ok_or("production line conversion control unreadable")?;
    if control == 0 {
        return Ok(false);
    }
    let variant = line
        .checked_add(LINE_VARIANT)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("production line design unreadable")?;
    crate::production_modifiers::conversion_target(memory, variant)
}

/// Console flag tested by naval and railway-gun production.
pub const INSTANT_ARMY_TRAINING: Rva = Rva(0x033d_da9b);

/// Classifies the already-applied CREDIT RDX input. Owner and producer must be
/// authenticated first. A refusal affects both spending fields, not the
/// independently paired delivery or MIO count. No game function is invoked.
pub fn applied_spending(
    memory: &impl Memory,
    base: usize,
    country: u32,
    producer: usize,
    speed: i64,
) -> Result<(i64, i64), &'static str> {
    if producer >= PRODUCERS.len() {
        return Err("production output producer unsupported");
    }
    let instant = engine_sites::at(base, crate::construction_ic_sites::INSTANT_CONSTRUCTION)
        .and_then(|address| read::<u8>(memory, address))
        .ok_or("production output instant flag unreadable")?
        != 0;
    if instant {
        return Ok((0, speed));
    }
    if matches!(producer, 1 | 2) {
        let instant_training = engine_sites::at(base, INSTANT_ARMY_TRAINING)
            .and_then(|address| read::<u8>(memory, address))
            .ok_or("production output instant army training flag unreadable")?
            != 0;
        if instant_training {
            let state = engine_sites::at(base, cgame_state::SINGLETON_RVA)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|&pointer| pointer != 0)
                .ok_or("production output instant viewing state unreadable or null")?;
            // Read the observer only when the playing reference is not signed-positive.
            let playing = state
                .checked_add(cgame_state::LOCAL_PLAYER_COUNTRY)
                .and_then(|address| read::<u32>(memory, address))
                .ok_or("production output instant viewing country unreadable")?;
            let viewing = if (playing as i32) > 0 {
                playing
            } else {
                state
                    .checked_add(cgame_state::OBSERVED_COUNTRY)
                    .and_then(|address| read::<u32>(memory, address))
                    .ok_or("production output instant viewing country unreadable")?
            };
            let applies = if country == viewing {
                true
            } else {
                // Compare raw country-link entries, including zero, without resolving
                // country objects or reading their array.
                let links = state
                    .checked_add(cgame_state::COUNTRY_LINKS)
                    .and_then(|address| Array::read(memory, address, 4))
                    .ok_or("production output instant country links unreadable")?;
                let country = links
                    .element(country, 4)
                    .and_then(|address| read::<u32>(memory, address))
                    .ok_or("production output instant country link unreadable")?;
                let viewing = links
                    .element(viewing, 4)
                    .and_then(|address| read::<u32>(memory, address))
                    .ok_or("production output instant viewing link unreadable")?;
                country == viewing
            };
            if applies {
                return Ok((0, speed));
            }
        }
    }
    Ok((speed, 0))
}

/// Failures retain their independently authenticated country/design scope.
/// Zero country or variant marks an unavailable scope, never a bound identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum RocketFailure {
    #[default]
    None,
    Country,
    Design,
    Line,
    Amount,
    Instant,
}

impl RocketFailure {
    pub fn reason(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Country => Some("rocket spending country unreadable or unsupported"),
            Self::Design => Some("rocket spending design unreadable or unsupported"),
            Self::Line => Some("rocket spending receiver unreadable or unsupported"),
            Self::Amount => Some("rocket spending input unreadable"),
            Self::Instant => Some("rocket spending instant operands unreadable"),
        }
    }

    pub fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::None,
            1 => Self::Country,
            2 => Self::Design,
            3 => Self::Line,
            4 => Self::Amount,
            _ => Self::Instant,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RocketTick {
    pub country: u32,
    pub variant: u64,
    pub spent_raw: i64,
    pub instant_raw: i64,
    pub failure: RocketFailure,
}

/// Copies the exact rocket-production input at entry without a line handle or
/// census. A null variant bypasses spending and records nothing. Independent
/// scope reads survive another operand's failure; no game function is invoked.
pub fn read_rocket_spending(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Option<RocketTick> {
    let field = |offset| pointer.checked_add(offset).filter(|_| pointer != 0);
    let variant = field(ROCKET_VARIANT)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|address| usize::try_from(address).ok());
    if variant == Some(0) {
        return None;
    }
    let country = field(ROCKET_STATUS)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|address| usize::try_from(address).ok())
        .and_then(|status| production_status_country(memory, base, status).ok());
    let design =
        variant.and_then(|variant| cequipment_variant::identity(memory, base, variant).ok());
    let mut tick = RocketTick {
        country: country.unwrap_or(0),
        variant: design.unwrap_or(0),
        ..RocketTick::default()
    };
    tick.failure = if country.is_none() {
        RocketFailure::Country
    } else if design.is_none() {
        RocketFailure::Design
    } else if engine_sites::at(base, ROCKET_VTABLE).is_none_or(|vtable| {
        field(0).and_then(|address| read::<u64>(memory, address)) != Some(vtable as u64)
    }) {
        RocketFailure::Line
    } else {
        RocketFailure::None
    };
    if tick.failure != RocketFailure::None {
        return Some(tick);
    }
    let Some(speed) = field(ROCKET_SPEED).and_then(|address| read::<i64>(memory, address)) else {
        tick.failure = RocketFailure::Amount;
        return Some(tick);
    };
    let instant = engine_sites::at(base, crate::construction_ic_sites::INSTANT_CONSTRUCTION)
        .and_then(|address| read::<u8>(memory, address));
    match instant {
        Some(0) => tick.spent_raw = speed,
        Some(_) => {
            let cost = variant
                .and_then(|variant| {
                    variant.checked_add(cequipment_variant::BUILD_COST_CACHE_CANDIDATE)
                })
                .and_then(|address| read::<i64>(memory, address));
            if let Some(cost) = cost {
                tick.instant_raw = speed.max(cost);
            } else {
                tick.failure = RocketFailure::Instant;
            }
        }
        None => tick.failure = RocketFailure::Instant,
    }
    Some(tick)
}

/// The producing country's raw reference, also for credit without delivery.
/// The line selects its production status, which selects the country whose
/// logistics status receives [`DELIVERY`]. No game function is called.
///
/// `line` stays alive until its production call returns. The line's, status's
/// and country's vtables, the line's handle, the country's nonzero id and its
/// status backlink are checked, as in the census readers.
pub fn producing_country(
    memory: &impl Memory,
    base: usize,
    line: usize,
) -> Result<u32, &'static str> {
    owning_country(
        memory,
        base,
        line,
        &[
            cmilitary_production_line::VTABLE_RVA,
            cnaval_production_line::VTABLE_RVA,
            crailway_gun_production_line::VTABLE_RVA,
            cship_refit_production_line::VTABLE_RVA,
        ],
    )
}

/// The raw reference of the country whose production status owns `line`, as [`producing_country`]
/// reads it, for a line whose vtable is one of `vtables` (RVAs): any production line keeps its
/// CProductionStatus (`cproduction_line::PRODUCTION_STATUS`), construction lines included.
pub fn owning_country(
    memory: &impl Memory,
    base: usize,
    line: usize,
    vtables: &[Rva],
) -> Result<u32, &'static str> {
    let mut identity = [0; 16];
    let mut status = [0; 8];
    if line == 0
        || memory.copy_scattered(
            [
                line,
                line.checked_add(cproduction_line::PRODUCTION_STATUS)
                    .unwrap_or(0),
            ],
            &mut [&mut identity, &mut status],
        ) != 2
    {
        return Err("production line owner unreadable");
    }
    let vtable = u64::from_le_bytes(identity[..8].try_into().unwrap()) as usize;
    if engine_sites::which(base, vtable, vtables).is_none() {
        return Err("production line vtable mismatch");
    }
    if u64::from_le_bytes(identity[8..].try_into().unwrap()) == 0 {
        return Err("production line handle is null");
    }
    production_status_country(memory, base, u64::from_le_bytes(status) as usize)
}

fn production_status_country(
    memory: &impl Memory,
    base: usize,
    status: usize,
) -> Result<u32, &'static str> {
    if status == 0
        || engine_sites::at(base, cproduction_status::VTABLE_RVA)
            .is_none_or(|vtable| read::<u64>(memory, status) != Some(vtable as u64))
    {
        return Err("production status vtable mismatch");
    }
    let country = status
        .checked_add(cproduction_status::COUNTRY)
        .and_then(|p| read::<u64>(memory, p))
        .filter(|&p| p != 0)
        .ok_or("production status country unreadable or null")? as usize;
    let mut country_identity = [0; 12];
    let mut backlink = [0; 8];
    if memory.copy_scattered(
        [
            country,
            country
                .checked_add(ccountry::PRODUCTION_STATUS)
                .unwrap_or(0),
        ],
        &mut [&mut country_identity, &mut backlink],
    ) != 2
    {
        return Err("production country unreadable");
    }
    if Some(u64::from_le_bytes(country_identity[..8].try_into().unwrap()) as usize)
        != engine_sites::at(base, ccountry::VTABLE_RVA)
    {
        return Err("production country vtable mismatch");
    }
    let id = u32::from_le_bytes(country_identity[ccountry::ID..].try_into().unwrap());
    if id == 0 {
        return Err("production country id is zero");
    }
    if u64::from_le_bytes(backlink) as usize != status {
        return Err("production country status mismatch");
    }
    Ok(id)
}

#[cfg(test)]
#[path = "line_output_sites/spending_tests.rs"]
mod spending_tests;

#[cfg(test)]
#[path = "line_output_sites/rocket_tests.rs"]
mod rocket_tests;

#[cfg(test)]
#[path = "line_output_sites/conversion_tests.rs"]
mod conversion_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const BASE: usize = 0x140000000;
    const LINE: usize = 0x100;
    const STATUS: usize = 0x400;
    const COUNTRY: usize = 0x800;

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; COUNTRY + ccountry::PRODUCTION_STATUS + 8];
        for (address, value) in [
            (LINE, BASE + cmilitary_production_line::VTABLE_RVA.0),
            (LINE + LINE_HANDLE, 0x11_0000_0021),
            (LINE + cproduction_line::PRODUCTION_STATUS, STATUS),
            (STATUS, BASE + cproduction_status::VTABLE_RVA.0),
            (STATUS + cproduction_status::COUNTRY, COUNTRY),
            (COUNTRY, BASE + ccountry::VTABLE_RVA.0),
            (COUNTRY + ccountry::ID, 5001),
            (COUNTRY + ccountry::PRODUCTION_STATUS, STATUS),
        ] {
            bytes[address..address + 8].copy_from_slice(&(value as u64).to_le_bytes());
        }
        bytes
    }

    fn country(bytes: &[u8], line: usize) -> Result<u32, &'static str> {
        producing_country(&SliceMemory { base: 0, bytes }, BASE, line)
    }

    #[test]
    fn producer_is_the_status_owner_without_a_variant_or_organisation() {
        for vtable in [
            cmilitary_production_line::VTABLE_RVA,
            cnaval_production_line::VTABLE_RVA,
            crailway_gun_production_line::VTABLE_RVA,
            cship_refit_production_line::VTABLE_RVA,
        ] {
            let mut bytes = fixture();
            bytes[LINE..LINE + 8].copy_from_slice(&((BASE + vtable.0) as u64).to_le_bytes());
            assert_eq!(country(&bytes, LINE), Ok(5001));
        }
    }

    #[test]
    fn producer_refuses_broken_identity_ownership_and_short_reads() {
        for (address, expected) in [
            (LINE, "production line vtable mismatch"),
            (LINE + LINE_HANDLE, "production line handle is null"),
            (
                LINE + cproduction_line::PRODUCTION_STATUS,
                "production status vtable mismatch",
            ),
            (STATUS, "production status vtable mismatch"),
            (
                STATUS + cproduction_status::COUNTRY,
                "production status country unreadable or null",
            ),
            (COUNTRY, "production country vtable mismatch"),
            (COUNTRY + ccountry::ID, "production country id is zero"),
            (
                COUNTRY + ccountry::PRODUCTION_STATUS,
                "production country status mismatch",
            ),
        ] {
            let mut bytes = fixture();
            bytes[address..address + 8].fill(0);
            assert_eq!(country(&bytes, LINE), Err(expected));
        }
        let bytes = fixture();
        for end in [
            LINE + 15,
            LINE + cproduction_line::PRODUCTION_STATUS + 7,
            STATUS + 7,
            STATUS + cproduction_status::COUNTRY + 7,
            COUNTRY + 11,
            bytes.len() - 1,
        ] {
            assert!(
                country(&bytes[..end], LINE).is_err(),
                "short read at {end:#x}"
            );
        }
        assert!(country(&bytes, 0).is_err());
        assert!(country(&bytes, usize::MAX - 8).is_err());
    }
}
