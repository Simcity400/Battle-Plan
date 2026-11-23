use super::{
    Capacities, Memory, ccharacter, ccharacter_manager, cunit_leader, read, unit_identity,
};
use collector_primitives::country_slots::{raise, rows};

static LIVE_ROLES: [std::sync::atomic::AtomicUsize; 4] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 4];
static LIVE_TRAITS: [std::sync::atomic::AtomicUsize; 2] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 2];

#[cfg(any(test, feature = "test-support"))]
pub(super) fn forget_live() {
    for hint in LIVE_ROLES.iter().chain(&LIVE_TRAITS) {
        hint.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}

pub(super) fn initial_roles(part: usize) -> usize {
    LIVE_ROLES[part].load(std::sync::atomic::Ordering::Relaxed)
}

pub(super) fn initial_traits() -> cunit_leader::TraitDemand {
    cunit_leader::TraitDemand {
        rows: LIVE_TRAITS[0].load(std::sync::atomic::Ordering::Relaxed),
        bytes: LIVE_TRAITS[1].load(std::sync::atomic::Ordering::Relaxed),
    }
}

pub(super) fn presize(
    capacity: &mut Capacities,
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<(), &'static str> {
    let roster = ccharacter_manager::Roster::read(memory, base, state)?;
    super::census::remember_roster(roster.count() as usize);
    if let Ok(demand) = engine_native::cadvisor::definitions_demand(memory, base, state) {
        let total_rows = demand[0].rows.saturating_add(demand[1].rows);
        let total_bytes = demand[0].bytes.saturating_add(demand[1].bytes);
        raise(&mut capacity.rows[2], rows(total_rows, 1));
        raise(&mut capacity.bytes[2], rows(total_bytes, 1));
        LIVE_ROLES[2].fetch_max(total_rows, std::sync::atomic::Ordering::Relaxed);
        super::advisors::remember_definition_demand(demand);
    }
    let leaders = engine_native_politics::ccountry_leader::walk(
        memory,
        base,
        state,
        engine_native_memory::walk::Buffer::Count,
    );
    let leaders = match leaders {
        engine_native_memory::walk::Outcome::Rows { len } => Ok(len),
        engine_native_memory::walk::Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("country leader demand overflow"),
        engine_native_memory::walk::Outcome::Unavailable { reason } => Err(reason),
    };
    if let Ok(leaders) = leaders {
        raise(&mut capacity.rows[1], rows(leaders, 1));
        LIVE_ROLES[1].fetch_max(leaders, std::sync::atomic::Ordering::Relaxed);
    }
    let mut units = 0_usize;
    let mut unit_bytes = 0_usize;
    let mut traits = cunit_leader::TraitDemand::default();
    let mut refusal = None;
    for index in 0..roster.count() {
        let unit = (|| {
            let view = roster.view(memory, index, [false, false, true, false])?;
            let (character, id) = view.reference();
            let pointer = character
                .checked_add(ccharacter::UNIT_LEADER)
                .and_then(|address| read::<u64>(&view, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .ok_or("character unit leader pointer unreadable")?;
            if pointer == 0 {
                return Ok(None);
            }
            Ok(Some((pointer, unit_identity(memory, pointer, id)?)))
        })();
        let (pointer, id) = match unit {
            Ok(Some(unit)) => unit,
            Ok(None) => continue,
            Err(reason) => {
                refusal.get_or_insert(reason);
                continue;
            }
        };
        units = units
            .checked_add(1)
            .ok_or("unit leader sizing count overflow")?;
        let text = (|| {
            let demand =
                cunit_leader::read_text(memory, pointer, engine_native_memory::walk::Buffer::Count);
            if let engine_native_memory::walk::Outcome::Unavailable { reason } = demand.outcome {
                return Err(reason);
            }
            unit_bytes = unit_bytes
                .checked_add(demand.required_bytes)
                .ok_or("unit leader text demand overflow")?;
            Ok(())
        })();
        if let Err(reason) = text {
            refusal.get_or_insert(reason);
        }
        let demand =
            cunit_leader::trait_demand(memory, base, pointer, id, false).and_then(|next| {
                traits = cunit_leader::TraitDemand {
                    rows: traits
                        .rows
                        .checked_add(next.rows)
                        .ok_or("unit leader trait sizing row count overflow")?,
                    bytes: traits
                        .bytes
                        .checked_add(next.bytes)
                        .ok_or("unit leader trait sizing byte count overflow")?,
                };
                Ok(())
            });
        if let Err(reason) = demand {
            refusal.get_or_insert(reason);
        }
    }
    raise(&mut capacity.rows[3], rows(units, 1));
    LIVE_ROLES[0].fetch_max(units, std::sync::atomic::Ordering::Relaxed);
    raise(&mut capacity.bytes[3], rows(unit_bytes, 1));
    LIVE_TRAITS[0].fetch_max(traits.rows, std::sync::atomic::Ordering::Relaxed);
    LIVE_TRAITS[1].fetch_max(traits.bytes, std::sync::atomic::Ordering::Relaxed);
    raise(&mut capacity.rows[4], rows(traits.rows, 1));
    raise(&mut capacity.bytes[4], rows(traits.bytes, 1));
    refusal.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests;
