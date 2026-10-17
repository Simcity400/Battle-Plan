//! The layer's arithmetic, in the game's fixed point (five decimals) and the order it takes.

use super::{ActivityDefines, CounterDefines, IntelDefines, OperationState};
use crate::map::raids::tooltips::intel as counter;
use replay::rows::{OperationRow, RaidIntelInputRow};
use viewer_foundation::map::native_text::COLOR;

/// One whole in fixed5.
pub const ONE: i64 = 100_000;

/// An activity level past the content's colours.
pub const MAGENTA: [f32; 4] = [1.0, 0.0, 1.0, 1.0];

/// The alignment that centres a window on its projected point on both axes.
const CENTRED: u8 = 0x12;

/// A fixed5 product, truncated toward zero.
pub fn times(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}

/// A fixed5 quotient, truncated toward zero; `None` where the division has no result.
pub fn divide(a: i64, b: i64) -> Option<i64> {
    a.wrapping_mul(ONE).checked_div(b)
}

/// Whether the icons show at eye height `height`: strictly below the cutoff.
pub fn visible(height: f32, defines: &IntelDefines) -> bool {
    height < defines.cutoff()
}

/// The container's top-left: centred on its projected point, each axis rounded half away from
/// zero, then moved by the define's truncated offset.
pub fn container_point(screen: [f32; 2], size: [i32; 2], offset: [i32; 2]) -> [i32; 2] {
    let point = viewer_foundation::map::icon_windows::point(screen, size, CENTRED);
    [
        point[0].wrapping_add(offset[0]),
        point[1].wrapping_add(offset[1]),
    ]
}

/// The activity level: it starts at zero and advances while the activity reaches the level's
/// threshold.
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

/// The shield's tint at `level`: the content's colour, or magenta past the colours.
pub fn activity_color(level: usize, defines: &ActivityDefines) -> [f32; 4] {
    defines.colors.get(level).copied().unwrap_or(MAGENTA)
}

/// Each counteroperative's weighted contribution to `row`'s country, strongest first.
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

/// The agency's counterintelligence rating: the base, the country's defense and the
/// counteroperatives' stacked sum, not below zero; zero without a strategic object.
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

/// The agency defense of a rating: the log factor times the natural logarithm of one plus the
/// rating, plus the rating over the divisor, a term skipped whose factor or divisor is zero.
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

/// A fixed5 value rounded to a whole number, halves away from zero.
pub fn round_whole(value: i64) -> i64 {
    if value < 0 {
        value.wrapping_sub(ONE / 2) / ONE
    } else {
        value.wrapping_add(ONE / 2) / ONE
    }
}

/// An operative's root-out efficiency before stacking: the base efficiency times one plus the
/// country's and the operative's factors and the nationality's, not below zero.
pub fn root_out_base(base: i64, country: i64, operative: i64, nationality: i64) -> i64 {
    times(
        base,
        ONE.wrapping_add(country)
            .wrapping_add(operative)
            .wrapping_add(nationality),
    )
    .max(0)
}

/// Root-out efficiencies after stacking among one owner's operatives in one state, by handle:
/// deduplicated, greatest first, ties to the greater unsigned (type, id), weighted by the
/// running product of the stacking factor.
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

/// Whole days since `start`, each hour's day truncated from the game's epoch.
pub fn elapsed_days(start: i32, hour: i32) -> i32 {
    let day = |hour: i32| hour.wrapping_sub(replay::day_night::EPOCH) / 24;
    day(hour).wrapping_sub(day(start))
}

/// The operation's state at `hour`.
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

/// The running operation's current phase index: the elapsed days times the phase count over
/// the duration, none at the start and end sentinels or past the count.
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

/// The circular progress's percent: the elapsed share of the duration.
pub fn phase_progress(elapsed: i32, duration: i32) -> i32 {
    let share = elapsed as f32 / duration as f32 * 100.0;
    if share.is_finite() { share as i32 } else { 0 }
}

/// The preparation value: the received share of the untimed requirements, less the timed
/// requirements' share of what preparation still lacks.
pub fn preparation(row: &OperationRow) -> Result<i64, String> {
    let (mut required, mut received, mut timed) = (0_i64, 0_i64, 0_i64);
    for requirement in &row.requirements {
        if requirement.uses_time == 0 {
            required = required.wrapping_add(requirement.amount);
            // The pool's stored total for the requirement's archetype; none is zero.
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

/// Whole percent of a fixed5 fraction.
pub fn percent(value: i64) -> String {
    installed_content::fixed5::fixed5_percent(value, 0, true)
}

/// Whole percent of a fixed5 value already in percent.
pub fn percent_points(value: i64) -> String {
    installed_content::fixed5::fixed5_percent(value, 0, false)
}

/// `text` in the colour `code`.
pub fn coloured(code: char, text: &str) -> String {
    format!("{COLOR}{code}{text}{COLOR}!")
}

/// A cooldown's remaining time: whole days in yellow with the day unit while one remains,
/// otherwise hours with the hour unit.
pub fn cooldown_text(remaining_hours: i32, day_unit: &str, hour_unit: &str) -> String {
    let days = remaining_hours / 24;
    if days >= 1 {
        coloured('Y', &format!("{days}{day_unit}"))
    } else {
        format!("{remaining_hours}{hour_unit}")
    }
}

/// The status frame of a build-network mission: 1 for a gain, 2 for a loss, 3 for none.
pub fn gain_frame(gain: i64) -> i32 {
    match gain.signum() {
        1 => 1,
        -1 => 2,
        _ => 3,
    }
}

/// The ideology group's share of the country's party popularity, as the popularity getter
/// gives it; zero without popularity.
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
