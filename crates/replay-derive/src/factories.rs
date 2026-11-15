//! A country's factory counts as the game's views show them, from its CountryFactories row: each
//! group's built total, its top bar's count and what is available. `cell` reads a column of the row,
//! none where it is not recorded.

/// The counts by mode index: `damaged` is the built factories the game counts damaged.
pub const MODES: [&str; 4] = ["free", "built", "topBar", "damaged"];

/// CountryFactories' columns as the schema numbers them: a group's columns from `GROUP * group`
/// (military 0, naval 1, civilian 2), and the civilian consumers.
pub const GROUP: usize = 11;
pub const TOTAL: usize = 1;
pub const DAMAGED: usize = 4;
pub const TO_OVERLORD: usize = 8;
pub const TO_OTHERS: usize = 10;
pub const CONSUMERS: [usize; 5] = [35, 39, 36, 37, 38];

/// The CountryFactories columns every count reads.
pub const READS: [usize; 17] = [
    TOTAL, DAMAGED, TO_OVERLORD, TO_OTHERS,
    GROUP + TOTAL, GROUP + DAMAGED, GROUP + TO_OVERLORD, GROUP + TO_OTHERS,
    2 * GROUP + TOTAL, 2 * GROUP + DAMAGED, 2 * GROUP + TO_OVERLORD, 2 * GROUP + TO_OTHERS,
    CONSUMERS[0], CONSUMERS[1], CONSUMERS[2], CONSUMERS[3], CONSUMERS[4],
];

/// A fixed5 factory total as every game view shows it, `(int)(value / 100000)`; none when it does
/// not fit.
fn shown(raw: i64) -> Option<i64> {
    i32::try_from(raw / 100_000).ok().map(i64::from)
}

/// Group `group`'s count under mode `mode` (an index of [`MODES`]): built is the game's Total; the
/// top bar's is built less damaged and given away; available is that less, for civilian factories,
/// consumer goods, contracts, national projects and licence and trade imports. None when a cell it
/// reads is not recorded.
pub fn count(cell: &dyn Fn(usize) -> Option<i64>, group: usize, mode: usize) -> Option<i64> {
    let base = group * GROUP;
    if mode == 3 {
        return shown(cell(base + DAMAGED)?);
    }
    let total = shown(cell(base + TOTAL)?)?;
    if mode == 1 {
        return Some(total);
    }
    let given = cell(base + TO_OVERLORD)? + cell(base + TO_OTHERS)?;
    let left = total - shown(cell(base + DAMAGED)?)? - given;
    if mode == 2 || group != 2 {
        return Some(left);
    }
    CONSUMERS.iter().try_fold(left, |left, &column| Some(left - cell(column)?))
}

/// Mode `mode` summed over the groups of `mask` (military 1, naval 2, civilian 4).
pub fn channel(cell: &dyn Fn(usize) -> Option<i64>, mode: usize, mask: usize) -> Option<i64> {
    (0..3).filter(|group| mask & (1 << group) != 0).try_fold(0i64, |sum, group| Some(sum + count(cell, group, mode)?))
}
