
use super::{ActivityDefines, CounterDefines, IntelDefines, OperationState};
use crate::map::raids::tooltips::intel as counter;
use replay_data::{rows::OperationRow, rows::RaidIntelInputRow};
use viewer_native_gui::native_text::COLOR;

pub const ONE: i64 = 100_000;

pub const MAGENTA: [f32; 4] = [1.0, 0.0, 1.0, 1.0];

const CENTRED: u8 = 0x12;

pub fn times(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}

pub fn divide(a: i64, b: i64) -> Option<i64> {
    a.wrapping_mul(ONE).checked_div(b)
}

pub fn visible(height: f32, defines: &IntelDefines) -> bool {
    height < defines.cutoff()
}

pub fn container_point(screen: [f32; 2], size: [i32; 2], offset: [i32; 2]) -> [i32; 2] {
    let point = viewer_native_gui::icon_windows::point(screen, size, CENTRED);
    [
        point[0].wrapping_add(offset[0]),
        point[1].wrapping_add(offset[1]),
    ]
}

pub fn activity_level(activity: i64, defines: &ActivityDefines) -> usize {
    let mut level = 0;
    while defines
        .thresholds
        .get(level)
        .is_some_and(|&threshold| activity >= threshold)
    {
        level += 1;
    }
    level
}

pub fn activity_color(level: usize, defines: &ActivityDefines) -> [f32; 4] {
    defines.colors.get(level).copied().unwrap_or(MAGENTA)
}

pub fn counter_contributions(
    row: &RaidIntelInputRow,
    d: &CounterDefines,
) -> Result<Vec<(u64, i64)>, String> {
    counter::contributions(
        &row.operatives,
        d.operative_base,
        d.stacking,
        d.foreign_agent,
    )
}

pub fn counter_rating(row: &RaidIntelInputRow, d: &CounterDefines) -> Result<i64, String> {
    match row.strategic_present {
        0 => Ok(0),
        1 => {
            let stacked = counter_contributions(row, d)?
                .into_iter()
                .fold(0_i64, |sum, (_, value)| sum.wrapping_add(value));
            Ok(d.base
                .wrapping_add(row.country_defense)
                .wrapping_add(stacked.max(0)))
        }
        _ => Err("RaidIntelInputs strategic_present is not a boolean".into()),
    }
}

pub fn agency_defense(rating: i64, d: &CounterDefines) -> Result<i64, String> {
    let mut defense = 0_i64;
    if d.log_factor != 0 {
        let argument = (ONE.wrapping_add(rating)) as f64 / ONE as f64;
        let logarithm = argument.ln() * ONE as f64;
        if !logarithm.is_finite() {
            return Err(format!(
                "the logarithm of one plus the rating {rating} has no fixed-point value"
            ));
        }
        defense = defense.wrapping_add(times(d.log_factor, logarithm as i64));
    }
    if d.divisor != 0 {
        defense = defense.wrapping_add(
            divide(rating, d.divisor).ok_or("the rating's division by the divisor overflows")?,
        );
    }
    Ok(defense)
}

pub fn round_whole(value: i64) -> i64 {
    if value < 0 {
        value.wrapping_sub(ONE / 2) / ONE
    } else {
        value.wrapping_add(ONE / 2) / ONE
    }
}

pub fn root_out_base(base: i64, country: i64, operative: i64, nationality: i64) -> i64 {
    times(
        base,
        ONE.wrapping_add(country)
            .wrapping_add(operative)
            .wrapping_add(nationality),
    )
    .max(0)
}

pub fn root_out_stacked(operatives: &[(u64, i64)], factor: i64) -> Vec<(u64, i64)> {
    let mut unique: Vec<(u64, i64)> = Vec::with_capacity(operatives.len());
    for &(handle, value) in operatives {
        if !unique.iter().any(|(seen, _)| *seen == handle) {
            unique.push((handle, value));
        }
    }
    let reference = |handle: u64| (handle as u32, (handle >> 32) as u32);
    unique.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| reference(b.0).cmp(&reference(a.0)))
    });
    let mut weight = ONE;
    let mut out = Vec::with_capacity(unique.len());
    for (index, (handle, value)) in unique.into_iter().enumerate() {
        out.push((
            handle,
            if index == 0 {
                value
            } else {
                times(value, weight)
            },
        ));
        weight = times(weight, factor);
    }
    out
}

pub fn elapsed_days(start: i32, hour: i32) -> i32 {
    let day = |hour: i32| hour.wrapping_sub(replay_data::day_night::EPOCH) / 24;
    day(hour).wrapping_sub(day(start))
}

pub fn operation_state(row: &OperationRow, hour: i32) -> OperationState {
    if row.terminal != 0 {
        OperationState::Finished
    } else if row.duration_days == 0 {
        OperationState::Preparing
    } else {
        let elapsed = elapsed_days(row.start_hour, hour);
        OperationState::Running {
            elapsed,
            remaining: row.duration_days.wrapping_sub(elapsed),
        }
    }
}

pub fn current_phase(row: &OperationRow, hour: i32) -> Option<usize> {
    let OperationState::Running { elapsed, .. } = operation_state(row, hour) else {
        return None;
    };
    if elapsed == -1 || elapsed == row.duration_days {
        return None;
    }
    let phase = elapsed
        .wrapping_mul(row.phase_count as i32)
        .checked_div(row.duration_days)?;
    u32::try_from(phase)
        .ok()
        .filter(|phase| *phase < row.phase_count)
        .map(|phase| phase as usize)
}

pub fn phase_progress(elapsed: i32, duration: i32) -> i32 {
    let share = elapsed as f32 / duration as f32 * 100.0;
    if share.is_finite() { share as i32 } else { 0 }
}

pub fn preparation(row: &OperationRow) -> Result<i64, String> {
    let (mut required, mut received, mut timed) = (0_i64, 0_i64, 0_i64);
    for requirement in &row.requirements {
        if requirement.uses_time == 0 {
            required = required.wrapping_add(requirement.amount);
            let pool = row
                .equipment_totals
                .iter()
                .find(|total| total.archetype == requirement.archetype)
                .map_or(0, |total| total.amount);
            received = received.wrapping_add(pool);
        } else {
            timed += 1;
        }
    }
    let ratio = if required <= 0 {
        ONE
    } else {
        divide(received, required).ok_or("the received share's division overflows")?
    };
    if row.preparation_current >= row.preparation_required {
        return Ok(ratio);
    }
    let time = divide(row.preparation_current, row.preparation_required)
        .ok_or("the preparation share's division has no result")?;
    let lacking = times(ratio, ONE.wrapping_sub(time)).wrapping_mul(timed);
    let count = (row.requirements.len() as i64).wrapping_mul(ONE);
    let share = divide(lacking, count)
        .ok_or("the timed share's division by the requirement count has no result")?;
    Ok(ratio.wrapping_sub(share))
}

pub fn percent(value: i64) -> String {
    content_reader_parser::fixed5::fixed5_percent(value, 0, true)
}

pub fn percent_points(value: i64) -> String {
    content_reader_parser::fixed5::fixed5_percent(value, 0, false)
}

pub fn coloured(code: char, text: &str) -> String {
    format!("{COLOR}{code}{text}{COLOR}!")
}

pub fn cooldown_text(remaining_hours: i32, day_unit: &str, hour_unit: &str) -> String {
    let days = remaining_hours / 24;
    if days >= 1 {
        coloured('Y', &format!("{days}{day_unit}"))
    } else {
        format!("{remaining_hours}{hour_unit}")
    }
}

pub fn gain_frame(gain: i64) -> i32 {
    match gain.signum() {
        1 => 1,
        -1 => 2,
        _ => 3,
    }
}

pub fn ideology_share(matching: i64, total: i64) -> i64 {
    if total == 0 {
        return 0;
    }
    if (matching as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        matching.wrapping_mul(ONE).wrapping_div(total)
    } else {
        0xffff_ffff
    }
}
