//! Province day/night clocks replayed from the recorded content stack, with no memory access: the
//! local hour and night factor held in each province slot of the weather manager's day/night array,
//! read by the province tooltip, combat and air spotting.
//!
//! Initialization builds each record's constants from the province's integer centroid, the map size
//! and four `NGraphics` defines ([`ProvinceClock::new`]). Each hourly update that finds both
//! suspension gates clear rebuilds the seasonal direction for the current date
//! ([`seasonal_direction`]) and updates every record from slot 1 ([`ProvinceClock::update`]). Slot
//! 0 is never updated, a suspended manager updates no slot, and a newly allocated record holds hour
//! 0 and night 100000 until its first update: those clocks are not a function of the hour and must
//! be recorded ([`crate::rows::DayNightClock`]).
//!
//! Every function preserves the game's integer arithmetic: 64-bit wrapping products,
//! truncating division and the fixed-point division guard that stores `0xFFFFFFFF` for a
//! numerator of `i64::MIN`, `i64::MIN + 1` or `i64::MAX`.

/// `fixed5` scale.
pub const SCALE: i64 = 100_000;
/// What the native fixed-point division stores when its guard refuses the numerator.
const REFUSED: i64 = 0xffff_ffff;
/// The date epoch the day/night update and air spotting subtract before taking an hour
/// index.
pub const EPOCH: i32 = 43_800_000;
/// Hourly vectors per record.
const HOURS: usize = 24;
/// Days in the game's non-leap year.
const DAYS_PER_YEAR: i32 = 365;

/// The division guard excludes the three sentinel numerator words.
fn divisible(numerator: i64) -> bool {
    (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

/// Scale the numerator before guarded division; clock initialization and seasonal
/// direction calculation also refuse a zero denominator before dividing.
fn fraction(numerator: i64, denominator: i64) -> i64 {
    if denominator == 0 || !divisible(numerator) {
        REFUSED
    } else {
        numerator.wrapping_mul(SCALE).wrapping_div(denominator)
    }
}

/// The `NGraphics` defines the clocks read, from the recorded content stack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Defines {
    /// `NGraphics.GMT_OFFSET`: a signed i32 map column.
    pub gmt_offset: i32,
    /// `NGraphics.SOUTH_POLE_OFFSET`, fixed5.
    pub south_pole_offset: i64,
    /// `NGraphics.NORTH_POLE_OFFSET`, fixed5.
    pub north_pole_offset: i64,
    /// `NGraphics.DAY_NIGHT_FEATHER`, fixed5.
    pub day_night_feather: i64,
}

impl Defines {
    /// Reads each define with the content stack's typed getters; the first failure refuses
    /// the set.
    pub fn read<E>(
        mut int32: impl FnMut(&'static str, &'static str) -> Result<i32, E>,
        mut fixed5: impl FnMut(&'static str, &'static str) -> Result<i64, E>,
    ) -> Result<Self, E> {
        Ok(Self {
            gmt_offset: int32("NGraphics", "GMT_OFFSET")?,
            south_pole_offset: fixed5("NGraphics", "SOUTH_POLE_OFFSET")?,
            north_pole_offset: fixed5("NGraphics", "NORTH_POLE_OFFSET")?,
            day_night_feather: fixed5("NGraphics", "DAY_NIGHT_FEATHER")?,
        })
    }
}

/// The clock a day/night record holds: the local hour and the night factor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    /// Signed i32 local hour; 1..=24 after an update of a centroid inside the map.
    pub local_hour: i32,
    /// Signed fixed5 night factor, clamped to 0..=100000 by every update.
    pub night_factor: i64,
}

impl Clock {
    /// The local-time tooltip's icon frame: round ties away from zero, take the low
    /// signed i32 of the result and add one.
    pub fn icon_frame(self) -> i32 {
        let half = if self.night_factor < 0 {
            -50_000
        } else {
            50_000
        };
        let rounded = self.night_factor.wrapping_add(half).wrapping_div(SCALE);
        (rounded as i32).wrapping_add(1)
    }
}

/// One initialized record's constants: 24 hourly sun vectors and the longitude phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProvinceClock {
    vectors: [[i64; 3]; HOURS],
    phase: i64,
}

impl ProvinceClock {
    /// Initialize a province whose definition's computed centroid is `centroid`, on a map of `map`
    /// columns and rows.
    pub fn new(centroid: [i32; 2], map: [i32; 2], defines: &Defines) -> Self {
        // Signed i32 subtraction before widening, then the guarded fixed division.
        let column = i64::from(centroid[0].wrapping_sub(defines.gmt_offset));
        let phase = fraction(
            column.wrapping_mul(SCALE),
            i64::from(map[0]).wrapping_mul(SCALE),
        )
        .wrapping_add(SCALE);
        let latitude = fraction(
            i64::from(centroid[1]).wrapping_mul(SCALE),
            i64::from(map[1]).wrapping_mul(SCALE),
        );
        let latitude = defines
            .north_pole_offset
            .wrapping_sub(defines.south_pole_offset)
            .wrapping_mul(latitude)
            / SCALE;
        let latitude = latitude.wrapping_add(defines.south_pole_offset);
        let vertical = cosine(latitude.wrapping_mul(314_159) / SCALE).wrapping_neg();
        let horizontal = SCALE.wrapping_sub(vertical.wrapping_abs());
        let mut vectors = [[0; 3]; HOURS];
        for (hour, vector) in vectors.iter_mut().enumerate() {
            let delta = hour_delta(hour as i64);
            let angle = (phase.wrapping_add(delta) % SCALE).wrapping_mul(628_318) / SCALE;
            *vector = normalize([
                horizontal.wrapping_mul(sine(angle)) / SCALE,
                vertical,
                cosine(angle).wrapping_mul(horizontal) / SCALE,
            ]);
        }
        Self { vectors, phase }
    }

    /// The record's clock after an update at `hour`.
    pub fn update(&self, hour: &HourOperands) -> Clock {
        let vector = &self.vectors[hour.index];
        let dot = hour
            .direction
            .iter()
            .zip(vector)
            .fold(0_i64, |sum, (a, b)| {
                sum.wrapping_add(a.wrapping_mul(*b) / SCALE)
            });
        let night = dot
            .wrapping_sub(hour.subtract)
            .wrapping_mul(hour.scale)
            .wrapping_div(SCALE)
            .clamp(0, SCALE);
        let local = (self.phase.wrapping_add(hour_delta(hour.index as i64)) % SCALE)
            .wrapping_mul(2_400_000);
        Clock {
            local_hour: ((local / 10_000_000_000) as i32).wrapping_add(1),
            night_factor: night,
        }
    }

    /// All native hourly factors under one held seasonal direction. Air
    /// fuel forecasts use these stored vectors, independently of the clock's
    /// last updated hour and of a mission's selected day/night period.
    pub fn forecast(&self, direction: [i64; 3], [subtract, scale]: [i64; 2]) -> [i64; HOURS] {
        self.vectors.map(|vector| {
            let dot = direction[0].wrapping_mul(vector[0]) / SCALE;
            let dot = dot.wrapping_add(direction[2].wrapping_mul(vector[2]) / SCALE);
            let dot = dot.wrapping_add(direction[1].wrapping_mul(vector[1]) / SCALE);
            dot.wrapping_sub(subtract)
                .wrapping_mul(scale)
                .wrapping_div(SCALE)
                .clamp(0, SCALE)
        })
    }
}

pub fn forecast_feather(feather: i64) -> [i64; 2] {
    let upper = feather.wrapping_mul(50_000) / SCALE;
    let subtract = upper.wrapping_neg();
    let width = upper.wrapping_sub(subtract);
    let scale = if width == 0 {
        REFUSED
    } else {
        (10_000_000_000_i128 / i128::from(width)) as i64
    };
    [subtract, scale]
}

/// The fixed5 share of a day at `hour`, guarded during initialization and updates.
fn hour_delta(hour: i64) -> i64 {
    if divisible(hour.wrapping_mul(SCALE)) {
        hour.wrapping_mul(10_000_000_000) / 2_400_000
    } else {
        REFUSED
    }
}

/// The operands the day/night update shares with every record at one date: the hour
/// index, the seasonal direction and the feathering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HourOperands {
    index: usize,
    direction: [i64; 3],
    /// Minus half the feather.
    subtract: i64,
    /// Fixed-point reciprocal of the feather width.
    scale: i64,
}

impl HourOperands {
    /// The update at native date `hour` (the game state's date). A date before the epoch gives a
    /// negative index, which would read before the record's vectors; it is refused.
    pub fn new(hour: i32, defines: &Defines) -> Result<Self, &'static str> {
        let index = hour.wrapping_sub(EPOCH) % HOURS as i32;
        let index = usize::try_from(index)
            .map_err(|_| "day/night hour index negative: date before the native epoch")?;
        let [subtract, scale] = forecast_feather(defines.day_night_feather);
        Ok(Self {
            index,
            direction: seasonal_direction(hour),
            subtract,
            scale,
        })
    }
}

/// The seasonal rotation about X in fixed-point degrees.
///
/// Construct June/December dates from the month-length table and set zero-based day 21:
/// the turns are day indices 172 and 355 (22 June and 22 December). Between them, scale
/// the elapsed fraction of 183 days by ten and add -500000. Across the year boundary,
/// subtract the elapsed fraction of 182 days from 50000, then scale by ten. Each elapsed
/// fraction uses fixed5 truncation.
pub fn seasonal_rotation(hour: i32) -> i64 {
    let day = hour.div_euclid(24).rem_euclid(DAYS_PER_YEAR);
    if (172..=355).contains(&day) {
        -500_000 + 10 * (i64::from(day - 172) * SCALE / 183)
    } else {
        let elapsed = if day < 172 { day + 10 } else { day - 355 };
        10 * (50_000 - i64::from(elapsed) * SCALE / 182)
    }
}

/// The seasonal direction the weather manager holds: it resets (0, 0, 100000), rotates about X and
/// clamps sine and cosine.
pub fn seasonal_direction(hour: i32) -> [i64; 3] {
    let radians = (seasonal_rotation(hour) * SCALE / 18_000_000) * 314_159 / SCALE;
    [
        0,
        sine(radians).clamp(-SCALE, SCALE),
        sine(radians + 157_080).clamp(-SCALE, SCALE),
    ]
}

/// Fixed-point sine using rounded quadrant constants and a three-term Taylor polynomial,
/// preserving each integer division.
pub fn sine(radians: i64) -> i64 {
    let phase = radians.rem_euclid(628_319);
    let mut x = if phase > 314_159 {
        phase - 314_159
    } else {
        phase
    };
    if x > 157_080 {
        x = 314_160 - x;
    }
    let cube = (x * x / SCALE) * x / SCALE;
    let fifth = (cube * x / SCALE) * x / SCALE;
    let sine = x - cube / 6 + fifth / 120;
    if phase > 314_159 { -sine } else { sine }
}

/// Fixed-point cosine: the sine a quarter turn later.
pub fn cosine(radians: i64) -> i64 {
    sine(radians.wrapping_add(157_080))
}

/// Normalize with an integer floor square root and a signed-i32 divisor.
fn normalize(vector: [i64; 3]) -> [i64; 3] {
    if vector == [0; 3] {
        return vector;
    }
    let square = vector.into_iter().fold(0_i64, |sum, value| {
        sum.wrapping_add(value.wrapping_mul(value))
    });
    let mut root = square;
    if square > 1 {
        let mut next = square / 2;
        loop {
            root = next;
            next = (square / root + root) / 2;
            if next >= root {
                break;
            }
        }
    }
    let divisor = i64::from(root as i32);
    if divisor == 0 {
        return [REFUSED; 3];
    }
    vector.map(|value| {
        if divisible(value) {
            value.wrapping_mul(SCALE).wrapping_div(divisor)
        } else {
            REFUSED
        }
    })
}

#[cfg(test)]
mod tests;
