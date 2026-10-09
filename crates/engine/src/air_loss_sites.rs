//! Native aircraft-loss bookkeeping at `140C4D180`, called by `140C4D420`.
//!
//! Writer hooks are chosen for per-event hour, region, family, cause and
//! opponent context. Differencing the current record between midnight reads
//! would be exact for totals when both advances are witnessed. The daily pass
//! `140C3A9C0` advances only the roster selected by `1406F35B0` (owned states
//! +484 or diplomacy +1A8); the event oracle checks that roster at both
//! midnights. Aggregate ordinals overlap, so totals cannot recover context.
//! `1415050B0` advances current slot +24, wraps at capacity +20 and zeroes
//! the new current record +30. Previous slot is `(current + 29) % 30` only
//! after a proved advance; +28 distinguishes the initial uncompleted slot.
//! `1401DCEF0` advances the date and completes hourly writes before daily
//! dispatch, so a normal midnight advance finalizes the bucket for `(H-24,H]`.
//! Air writers obtain only +10/+30 through `141504620`; roll-up helpers
//! `141504DD0/141504F50` write higher windows rather than completed daily
//! records. Attach/load (`140DC2220` also calls DailyUpdate) and partial-day
//! country-list membership prevent assigning every prior ring slot a date.
//!
//! `140C4D180` writes `loss_count * 100000` twice: the selected family total
//! and that total's index plus the raw cause. Family bases are own/enemy
//! fighter 3/44, bomber 10/51, CAS 17/58, transport 24/65, scout 30/71 and
//! missile 40/81. Only these two words are read; family/cause metadata retains
//! their context despite overlapping ordinals. An unknown wing uses base zero.
//! Complete loss-field names from `140C4D180`'s family selection and the
//! cause writers below (each cell is own/credited field):
//!
//! | Family | Total | Aircraft combat | AA/raid effect | Accident | Airbase bombing | Train AA | Naval kamikaze AA |
//! |---|---|---|---|---|---|---|---|
//! | Fighter | 3/44 | 4/45 | 5/46 | 6/47 | 7/48 | 8/49 | 9/50 |
//! | Bomber | 10/51 | 11/52 | 12/53 | 13/54 | 14/55 | 15/56 | 16/57 |
//! | CAS | 17/58 | 18/59 | 19/60 | 20/61 | 21/62 | 22/63 | 23/64 |
//! | Transport | 24/65 | 25/66 | 26/67 | 27/68 | 28/69 | 29/70 | 30/71 |
//! | Scout | 30/71 | 31/72 | 32/73 | 33/74 | 34/75 | 35/76 | 36/77 |
//! | Missile | 40/81 | 41/82 | 42/83 | 43/84 | 44/85 | 45/86 | 46/87 |
//!
//! Cells overlap by native design. In particular transport cause6 overlaps
//! scout total, scout cause6 overlaps trains, and missile causes overlap other
//! fields. A field index alone has no unique event meaning: the recorded
//! selected family base, cause and own/credited flag supply its context.
//! Even differencing a continuing daily bucket cannot recover that context;
//! the read-only ring cannot separate these loss families and other channels.
//! `140C4D620` writes building bombing to 0/41, so field41 in a loss event
//! has missile-cause context instead of certifying building damage.
//! Cause 3 is omitted on the credited side; the own side still includes it.
//! Causes remain raw native integers: 1 aircraft combat (`140F71120`, naval
//! `141967CD0`); 2 anti-air (`141295D10`, `140F718C0`, `141969200`) or raid
//! damage effect (`141454710`); 3 accident (`140F48C00`); 4 aircraft destroyed
//! by bombing their airbase (`140C4BC40`); 5 train anti-air (`140F60880`);
//! 6 naval anti-air against kamikaze (`141969200`, selected by wing +94 bit7
//! and AIR_NAVAL_KAMIKAZE_LOSSES_MULT, forwarded by `141966B10`). None is
//! relabeled as a generic shot-down count by this reader.
//! The divisor for aircraft counts is 100000. Values remain signed i64.
//! The count argument is native bookkeeping, not an independent removal
//! witness: `140F4E400` consumes a requested count until its variant bank is
//! exhausted and then recounts the wing; callers pass the original request
//! to `140C4D420`. A saturated counter cannot certify the missing amount.
//! `140C4BC40` also removes aircraft after a no-wing building-health decrease
//! but invokes bookkeeping only for a nonnull attacking wing. These counters
//! therefore preserve native loss statistics, not every inventory removal.
//! Collector validation therefore refuses increments that differ from the
//! two expected signed-count writes, including a positive request at the
//! clamp. `141504630` does not validate the index; collectors must do so.
//!
//! Synthetic tests establish guarded layouts and exact arithmetic. Live hook
//! behavior, fresh-save agreement and gameplay cost remain unverified.

use crate::{
    Memory, cair_wing, cair_wing_pool, ccountry, cloop_history, cpdx_array::Array, cstrategic_air,
    csub_unit_definition, loop_history_record_fields, loop_history_window, read,
};

/// Seven-argument writer: manager entry, wing, count, enemy, region, selection, cause.
pub const WRITER_RVA: usize = 0x00c4_d180;
/// The air-specific constructor `140C40870` passes 0x59 to `141504330`.
pub const FIELD_COUNT: usize = cloop_history::air_stats::FIELD_COUNT as usize;
/// `140C4D180` family selection, including its unknown-family base zero.
pub const FAMILY_BASES: [u32; 13] = [0, 3, 10, 17, 24, 30, 40, 44, 51, 58, 65, 71, 81];

pub struct Site {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// Both direct calls in the writer's only caller `140C4D420`. Tapping these
/// CALLs preserves the native writer's prologue and system unwind metadata.
/// The own call returns at `140C4D49E`; credit returns at `140C4D4EE`.
pub const SITES: [Site; 2] = [
    Site {
        name: "own_air_loss",
        rva: 0x00c4_d499,
        sentinel: [
            0xe8, 0xe2, 0xfc, 0xff, 0xff, 0x8b, 0x06, 0x8b, 0x17, 0x3b, 0xc2, 0x74, 0x48, 0x85,
            0xc0, 0x74,
        ],
    },
    Site {
        name: "credited_air_loss",
        rva: 0x00c4_d4e9,
        sentinel: [
            0xe8, 0x92, 0xfc, 0xff, 0xff, 0x48, 0x8b, 0xcf, 0xe8, 0x3a, 0x9d, 0x06, 0x00, 0x45,
            0x85, 0xff,
        ],
    },
];

/// Read the exact family-base selection in `140C4D180`, in its native order.
/// The masks are executable predicate facts, independent of content names.
/// A fully readable unknown native family returns base zero, which the native
/// writer also uses when its assertion does not terminate execution.
pub fn selected_family(
    memory: &impl Memory,
    base: usize,
    wing: usize,
    enemy: u8,
) -> Result<u32, &'static str> {
    if enemy > 1 {
        return Err("air loss writer enemy flag is not boolean");
    }
    authenticate(memory, base, wing, cair_wing::VTABLE_RVA)?;
    let pool = pointer(memory, wing, cair_wing::POOL)?;
    authenticate(memory, base, pool, cair_wing_pool::VTABLE_RVA)?;
    let definition = pointer(memory, pool, cair_wing_pool::DEFINITION)?;
    let vtable = read::<u64>(memory, definition).ok_or("air loss definition vtable unreadable")?;
    if ![
        csub_unit_definition::VTABLE_RVA,
        csub_unit_definition::NULL_VTABLE_RVA,
    ]
    .into_iter()
    .any(|rva| base.checked_add(rva).map(|address| address as u64) == Some(vtable))
    {
        return Err("air loss definition form unsupported");
    }
    let flags = field::<u64>(memory, definition, csub_unit_definition::AIR_FLAGS)?;
    Ok(family_base(flags, enemy))
}

fn family_base(flags: u64, enemy: u8) -> u32 {
    if flags & 0x1_0000_0c00 != 0 {
        // fighter-family predicate `140F4DF80`.
        [3, 44][enemy as usize]
    } else if flags & 0x0020_b000 != 0 || flags & (1 << 17) != 0 {
        // `140F4DE80`, `140F4DFA0`.
        [10, 51][enemy as usize]
    } else if flags & (1 << 14) != 0 {
        // CAS predicate `140F4DEA0`.
        [17, 58][enemy as usize]
    } else if flags & (1 << 20) != 0 {
        // transport predicate `140F4E3C0`.
        [24, 65][enemy as usize]
    } else if flags & (1 << 18) != 0 {
        // scout predicate `140F4E3A0`.
        [30, 71][enemy as usize]
    } else if flags & 0x1c_0001_0000 != 0 {
        // missile-family predicate `140F4DFC0`.
        [40, 81][enemy as usize]
    } else {
        0 // `140C4D180` initializes iVar7 to zero before all predicates.
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub country: u32,
    pub region: u32,
    pub enemy: u8,
    pub cause: i32,
    pub family_base: u32,
    pub fields: [i64; 2],
    receiver: usize,
    history: usize,
    record: usize,
    slot: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub country: u32,
    pub region: u32,
    /// Authenticated post-call daily cursor (`141504620`, `1415050B0`).
    /// Includes a history newly allocated by this call.
    pub daily_slot: u32,
    pub enemy: u8,
    pub cause: i32,
    /// Actual after-minus-before native fields, including native clamp effects.
    pub fields: [i64; 2],
}

/// Called immediately before the native writer. A missing history is a known
/// zero only here: `140C40870` constructs and zeroes its current record before
/// the first add. A null region or credited accident performs no field write.
#[allow(clippy::too_many_arguments)]
pub fn before(
    memory: &impl Memory,
    base: usize,
    game: usize,
    receiver: usize,
    region: u32,
    enemy: u8,
    cause: i32,
    family_base: u32,
) -> Result<Snapshot, &'static str> {
    if enemy > 1 {
        return Err("air loss writer enemy flag is not boolean");
    }
    selected_indices(family_base, cause)?;
    let mut snapshot = Snapshot {
        country: 0,
        region,
        enemy,
        cause,
        family_base,
        fields: [0; 2],
        receiver,
        history: 0,
        record: 0,
        slot: 0,
    };
    snapshot.country = owner(memory, base, game, receiver)?;
    snapshot.history = history(memory, receiver, snapshot.region)?;
    if snapshot.history != 0 {
        let current = current(memory, base, snapshot.history, family_base, cause)?;
        snapshot.record = current.record;
        snapshot.slot = current.slot;
        snapshot.fields = current.fields;
    }
    Ok(snapshot)
}

/// Called immediately after the same synchronous native writer returns.
/// An identity or current-record change refuses the event instead of assigning
/// an unrelated record's values to the writer. This function allocates nothing.
pub fn after(
    memory: &impl Memory,
    base: usize,
    game: usize,
    before: &Snapshot,
) -> Result<Change, &'static str> {
    let mut change = Change {
        country: before.country,
        region: before.region,
        daily_slot: 0,
        enemy: before.enemy,
        cause: before.cause,
        fields: [0; 2],
    };
    if owner(memory, base, game, before.receiver)? != before.country {
        return Err("air loss writer country changed during call");
    }
    let history = history(memory, before.receiver, before.region)?;
    if history == 0 || (before.history != 0 && history != before.history) {
        return Err("air loss writer history changed during call");
    }
    let current = current(memory, base, history, before.family_base, before.cause)?;
    if before.history != 0 && (before.record != current.record || before.slot != current.slot) {
        return Err("air loss writer current record changed during call");
    }
    change.daily_slot = current.slot;
    for ((delta, after), before) in change
        .fields
        .iter_mut()
        .zip(current.fields)
        .zip(before.fields)
    {
        *delta = after
            .checked_sub(before)
            .ok_or("air loss writer delta overflow")?;
    }
    Ok(change)
}

/// Native `140C4D180` writes family and family+cause, including twice for cause zero.
pub fn selected_indices(family: u32, cause: i32) -> Result<[u32; 2], &'static str> {
    let second = i64::from(family) + i64::from(cause);
    if family as usize >= FIELD_COUNT || !(0..FIELD_COUNT as i64).contains(&second) {
        return Err("air loss cause field is outside native record");
    }
    Ok([family, second as u32])
}

/// `140C4D43F` retains the credited tag pointer in RDI; `140C4D457` retains
/// GetEffectiveCountryTag's result in RSI. Neither changes before either tap.
/// Only the opposite tag word is followed, using the native signed-positive gate.
pub fn opponent(
    memory: &impl Memory,
    tags: [usize; 2],
    enemy: u8,
) -> Result<Option<u32>, &'static str> {
    let pointer = tags[usize::from(enemy == 0)];
    let tag = read::<i32>(memory, pointer).ok_or("air loss opponent tag unreadable")?;
    Ok((tag > 0).then_some(tag as u32))
}

fn owner(
    memory: &impl Memory,
    base: usize,
    game: usize,
    receiver: usize,
) -> Result<u32, &'static str> {
    authenticate(memory, base, receiver, cstrategic_air::VTABLE_RVA)?;
    let country = field(memory, receiver, cstrategic_air::COUNTRY)?;
    ccountry::Reader::new(memory, base, game)?.resolve(country)?;
    Ok(country)
}

fn history(memory: &impl Memory, receiver: usize, region: u32) -> Result<usize, &'static str> {
    let histories = array(memory, receiver, cstrategic_air::HISTORY_TABLE)?;
    let slot = histories
        .element(region, 8)
        .ok_or("air loss writer region outside history table")?;
    pointer(memory, slot, 0)
}

struct Current {
    record: usize,
    slot: u32,
    fields: [i64; 2],
}

fn current(
    memory: &impl Memory,
    base: usize,
    history: usize,
    family: u32,
    cause: i32,
) -> Result<Current, &'static str> {
    authenticate(memory, base, history, cloop_history::VTABLE_RVA)?;
    if pointer(memory, history, cloop_history::SELF_REFERENCE)? != history {
        return Err("air loss writer history self reference mismatch");
    }
    if field::<u32>(memory, history, cloop_history::FIELD_COUNT)? != FIELD_COUNT as u32 {
        return Err("air loss writer history field count mismatch");
    }
    let window = pointer(memory, history, cloop_history::DAILY_WINDOW)?;
    authenticate(memory, base, window, loop_history_window::VTABLE_RVA)?;
    let records = array(memory, window, loop_history_window::RECORDS)?;
    let capacity = field::<u32>(memory, window, loop_history_window::CAPACITY)?;
    let slot = field::<u32>(memory, window, loop_history_window::CURRENT_SLOT)?;
    let wrapped = field::<u8>(memory, window, loop_history_window::WRAPPED)?;
    if capacity != cloop_history::DAILY_WINDOW_CAPACITY
        || records.count != capacity
        || slot >= capacity
        || wrapped > 1
    {
        return Err("air loss writer daily window is invalid");
    }
    if pointer(memory, window, loop_history_window::HISTORY)? != history {
        return Err("air loss writer daily window owner mismatch");
    }
    let record = pointer(memory, window, loop_history_window::RECORD_FIELDS)?;
    let slot_address = records
        .element(slot, 8)
        .ok_or("air loss writer current slot address overflow")?;
    if record == 0 || pointer(memory, slot_address, 0)? != record {
        return Err("air loss writer current record disagrees with slot");
    }
    if pointer(memory, record, loop_history_record_fields::HISTORY)? != history {
        return Err("air loss writer record owner mismatch");
    }
    let values = array(memory, record, loop_history_record_fields::VALUES)?;
    if values.count != FIELD_COUNT as u32 {
        return Err("air loss writer record field count mismatch");
    }
    let indices = selected_indices(family, cause)?;
    let mut fields = [0; 2];
    for (value, index) in fields.iter_mut().zip(indices) {
        let address = values
            .element(index, 8)
            .ok_or("air loss selected field outside record")?;
        *value = read(memory, address).ok_or("air loss selected field unreadable")?;
    }
    Ok(Current {
        record,
        slot,
        fields,
    })
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    rva: usize,
) -> Result<(), &'static str> {
    let expected = base
        .checked_add(rva)
        .ok_or("air loss writer vtable address overflow")?;
    if object == 0 || read::<u64>(memory, object) != Some(expected as u64) {
        return Err("air loss writer object vtable unreadable or mismatched");
    }
    Ok(())
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|address| read(memory, address))
        .ok_or("air loss writer field unreadable or address overflow")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .map_err(|_| "air loss writer pointer outside address space")
}

fn array(memory: &impl Memory, object: usize, offset: usize) -> Result<Array, &'static str> {
    object
        .checked_add(offset)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("air loss writer array unreadable or invalid")
}

/// Queue sizing input only. Native `CStrategicAir+58` pools contain their
/// `CAirWing*` vector at +28/count +34 (`140C3A9C0`, `140C40900`). Read every
/// country's headers, including countries omitted from the native daily pass.
/// A malformed header refuses the sizing pass; no wing is followed and no
/// player/AI filter applies. This is not an event-count completeness bound.
pub fn wing_count(memory: &impl Memory, base: usize, game: usize) -> Result<usize, &'static str> {
    let mut total = 0_usize;
    cstrategic_air::visit(memory, base, game, |root| {
        let pools = array(memory, root.address, cstrategic_air::POOLS)?;
        crate::cpdx_array::pointers(
            memory,
            pools,
            "air loss wing pool pointer unreadable",
            |_, pool| {
                if pool == 0 {
                    return Err("air loss wing pool is null");
                }
                authenticate(memory, base, pool, cair_wing_pool::VTABLE_RVA)?;
                let wings = array(memory, pool, cair_wing_pool::WINGS)?;
                total = total
                    .checked_add(wings.count as usize)
                    .ok_or("air loss wing count overflow")?;
                Ok(())
            },
        )
    })?;
    Ok(total)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RosterRow {
    pub country: u32,
    pub eligible: bool,
}

/// `1401DEF80` builds CGameState+9A0's signed country-slot array through
/// `1406F35B0` (owned states +484 > 0 or diplomacy +F88 -> +1A8 > 0).
/// `1401D4390 -> 140C40900 -> 140C3A9C0` consumes this exact retained roster.
/// Read its membership, rather than recomputing a predicate after other daily
/// work could change it. Every canonical country gets a true/false witness.
pub fn roster(
    memory: &impl Memory,
    base: usize,
    game: usize,
    output: &mut Vec<RosterRow>,
) -> Result<(), &'static str> {
    output.clear();
    let countries = ccountry::Reader::new(memory, base, game)?;
    let mut selected = crate::allocation::filled(countries.count() as usize, false)
        .map_err(|_| "air daily roster allocation failed")?;
    let slots = countries.arrays()[0];
    let active = game
        .checked_add(crate::cgame_state::DAILY_COUNTRY_ROSTER)
        .and_then(|a| Array::read(memory, a, 4))
        .ok_or("air daily roster header invalid")?; // `1401DEF80`.
    for ordinal in 0..active.count {
        let address = active
            .element(ordinal, 4)
            .ok_or("air daily roster address overflow")?;
        let slot = read::<i32>(memory, address)
            .filter(|slot| *slot > 0)
            .ok_or("air daily roster country slot invalid")?;
        let address = slots
            .element(slot as u32, 8)
            .ok_or("air daily roster country slot outside country table")?;
        let country = countries.of_pointer(pointer(memory, address, 0)?)?;
        selected[country.canonical_slot as usize] = true;
    }
    crate::allocation::reserve_exact(output, countries.count() as usize)
        .map_err(|_| "air daily roster allocation failed")?;
    for slot in 1..countries.count() {
        if let Some(country) = countries.at(slot)? {
            output.push(RosterRow {
                country: country.id,
                eligible: selected[slot as usize],
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
