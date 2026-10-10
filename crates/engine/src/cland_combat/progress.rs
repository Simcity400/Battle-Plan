use super::{Reader, Source};
use crate::{Memory, carmy, cprovince, cunit};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProgressRow {
    pub id: u64,
    pub attacker_progress: i64,
    pub defender_progress: i64,
    /// First attacker with a nonnull map entity, or zero when absent.
    /// This supplies a province-direction operand, not a camera angle.
    pub direction_attacker: u64,
    pub direction_province: u32,
}

/// Exact qword organisation-progress arithmetic shared by ordinary
/// and border-war combat.
pub fn progress_from_organisation(attacker: i64, defender: i64) -> Option<i64> {
    let total = attacker.wrapping_add(defender);
    if total < 1 {
        return Some(50_000);
    }
    if (attacker as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        return Some(100_000);
    }
    attacker
        .wrapping_mul(100_000)
        .checked_div(total)
        .map(|v| v.clamp(0, 100_000))
}

/// Use the authenticated census without scanning the registry again or
/// calling the game. Sum organisation of all ordered side units, reserves
/// included. Defender viewpoint complements the fraction. Direction uses
/// the first attacker with a nonnull map entity. Process pointers and
/// camera/GUI state remain outside the recording.
pub fn read_progress(
    memory: &impl Memory,
    base: usize,
    source: Source,
) -> Result<ProgressRow, &'static str> {
    let reader = Reader {
        memory,
        base,
        provinces: source.provinces,
    };
    let mut sums = [0_i64; 2];
    let mut row = ProgressRow {
        id: source.id,
        ..ProgressRow::default()
    };
    for (side, units) in [source.attackers, source.defenders].into_iter().enumerate() {
        for member in 0..units.count {
            let pointer = reader.element(units, member)?;
            let id = reader.unit(pointer)?;
            sums[side] =
                sums[side].wrapping_add(reader.field::<i64>(pointer, carmy::ORGANISATION)?);
            if side == 0
                && row.direction_attacker == 0
                && reader.pointer(pointer, cunit::MAP_ENTITY)? != 0
            {
                let province = reader.pointer(pointer, cunit::LOCATION)?;
                reader.vtable(province, 0, cprovince::VTABLE_RVA)?;
                let province_id = reader.field::<u32>(province, cprovince::ID)?;
                if province_id == 0 || reader.element(source.provinces, province_id)? != province {
                    return Err("combat direction province differs from registered slot");
                }
                row.direction_attacker = id;
                row.direction_province = province_id;
            }
        }
    }
    row.attacker_progress = progress_from_organisation(sums[0], sums[1])
        .ok_or("combat progress native division overflow")?;
    row.defender_progress = 100_000 - row.attacker_progress;
    Ok(row)
}
