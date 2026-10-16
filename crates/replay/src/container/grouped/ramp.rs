//! Rate prediction for the numeric fields of ramped blocks
//! ([`super::Layout::Ramped`]).
//!
//! A drifting value stored at a display step ([`crate::display`]) moves by a
//! fractional number of steps an hour: a division regaining 0.13
//! organisation an hour, stored in tenths, changes by 1, 1, 2, 1, 1, 1, 2, ...;
//! an acclimatization factor falling 7.5 units an hour by -8, -7, -8, ...; a
//! remaining supply grace stored at whole percents of its share by one
//! percent every fifth hour. After `t` hours such a value has moved
//! `floor(phase + rate * t)` units of the greatest common divisor of its
//! changes, for some phase in `[0, 1)` and rate. [`Ramp`] keeps a field's
//! last [`WINDOW`] changes, its unchanged hours included, and the open
//! interval of rates every pair of them allows; it predicts the next change
//! with the simplest rate (the fraction with the smallest denominator) in
//! that interval and the middle of the phases that rate allows. A constant
//! change is an integer rate, an alternating one a half.
//!
//! A change no rate in the window allows is taken as a one-off (a wrapping
//! clock, a reset countdown) when the window predicted one: the window holds
//! the predicted change for that hour instead and remembers the actual one.
//! A second such change in a row is a new rate, and the window starts again
//! at the two. A field unchanged for [`IDLE`] hours, or whose change leaves
//! 32 bits, is forgotten (its repeated change still predicts it). Nothing
//! is predicted before a window holds [`EVIDENCE`] nonzero changes, so a
//! one-off change predicts nothing. Writer and reader compute the same exact
//! fractions from the same changes.

use std::cmp::Ordering;

/// Changes a ramp keeps.
pub(super) const WINDOW: usize = 32;
/// Changes a full window keeps when it takes the next one.
const KEEP: usize = 24;
/// Unchanged hours after which a field is forgotten.
pub(super) const IDLE: u8 = 8;
/// Nonzero changes a window holds before it predicts.
pub(super) const EVIDENCE: usize = 2;

/// An exact fraction with a positive denominator. A window's positions stay
/// within 32 * 2^31 units and its rates' denominators within 64, so every
/// product below fits 64 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Ratio {
    num: i64,
    den: i64,
}

impl Ratio {
    fn new(num: i64, den: i64) -> Self {
        debug_assert!(den > 0);
        Self { num, den }
    }

    fn integer(value: i64) -> Self {
        Self { num: value, den: 1 }
    }

    fn floor(self) -> i64 {
        self.num.div_euclid(self.den)
    }

    fn minus(self, value: i64) -> Self {
        Self::new(self.num - value * self.den, self.den)
    }

    /// `1 / self` of a positive fraction.
    fn reciprocal(self) -> Self {
        debug_assert!(self.num > 0);
        Self::new(self.den, self.num)
    }

    fn compare(self, other: Self) -> Ordering {
        (self.num * other.den).cmp(&(other.num * self.den))
    }
}

/// The fraction with the smallest denominator strictly between `low` and
/// `high` (unbounded above when None): the continued-fraction descent.
fn simplest(low: Ratio, high: Option<Ratio>) -> Ratio {
    let floor = low.floor();
    let next = Ratio::integer(floor + 1);
    let Some(high) = high.filter(|high| next.compare(*high) != Ordering::Less) else {
        return next;
    };
    // No integer between: floor <= low < high <= floor + 1, and x in
    // (low, high) is floor + 1 / y with y in (1 / (high - floor), 1 / (low - floor)).
    let (low, high) = (low.minus(floor), high.minus(floor));
    let y = simplest(high.reciprocal(), (low.num != 0).then(|| low.reciprocal()));
    Ratio::new(y.den + floor * y.num, y.num)
}

/// The greatest common divisor of two values.
fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// One numeric field's recent changes and the rates they allow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ramp {
    /// The field's position in units before the first held change and after
    /// each; the first `len + 1` are held.
    positions: [i64; WINDOW + 1],
    len: u8,
    /// The greatest common divisor of the held nonzero changes, at most 2^31.
    unit: i64,
    /// Trailing unchanged hours.
    idle: u8,
    /// Held nonzero changes.
    nonzero: u8,
    /// The open interval of rates, in units an hour, every pair of held
    /// changes allows; kept only from [`EVIDENCE`] nonzero changes on (a
    /// window with fewer always allows some rate).
    low: Ratio,
    high: Ratio,
    /// The predicted next change, zero for none.
    next: i64,
    /// The rate `next` was predicted with and the phases (times its
    /// denominator) it allows, while the window predicts: a narrower
    /// interval that still holds the rate has it as its simplest too, so an
    /// hour that keeps it costs one more phase bound.
    rated: Option<(Ratio, [i64; 2])>,
    /// The last hour's change when no rate allowed it, which the window
    /// holds as predicted instead.
    broken: Option<i32>,
}

impl std::hash::Hash for Ramp {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Reuse the maintained bounds, phase and endpoint rather than hashing
        // every position again. Equality still checks the complete window,
        // including positions outside the held prefix, on a matching hash.
        (
            self.len,
            self.unit,
            self.idle,
            self.nonzero,
            self.low,
            self.high,
            self.next,
            self.rated,
            self.broken,
            self.positions[usize::from(self.len)],
        )
            .hash(state);
    }
}

/// A window after one more hour.
// Returned once a field an hour and consumed at once; boxing the kept window
// would allocate per field per hour to shrink a value that never rests.
#[allow(clippy::large_enum_variant)]
enum Appended {
    Kept(Ramp),
    /// No rate allows every held change.
    Broken,
    Forgotten,
}

impl Ramp {
    /// A field's first change in a window; None for a change that is zero
    /// or leaves 32 bits.
    pub(super) fn start(change: i128) -> Option<Self> {
        let change = i32::try_from(change).ok().filter(|change| *change != 0)?;
        let mut ramp = Self {
            positions: [0; WINDOW + 1],
            len: 1,
            unit: i64::from(change.unsigned_abs()),
            idle: 0,
            nonzero: 1,
            low: Ratio::integer(0),
            high: Ratio::integer(0),
            next: 0,
            rated: None,
            broken: None,
        };
        ramp.positions[1] = change.signum().into();
        Some(ramp)
    }

    /// The predicted next change, zero for none.
    pub(super) fn next(&self) -> i128 {
        i128::from(self.next)
    }

    fn held(&self) -> &[i64] {
        &self.positions[..=usize::from(self.len)]
    }

    fn feasible(&self) -> bool {
        usize::from(self.nonzero) < EVIDENCE || self.low.compare(self.high) == Ordering::Less
    }

    /// Narrow the rate interval by the pairs ending at position `last`.
    fn narrow(&mut self, last: usize) {
        for first in 0..last {
            let span = (last - first) as i64;
            let moved = self.positions[last] - self.positions[first];
            let low = Ratio::new(moved - 1, span);
            let high = Ratio::new(moved + 1, span);
            if low.compare(self.low) == Ordering::Greater {
                self.low = low;
            }
            if high.compare(self.high) == Ordering::Less {
                self.high = high;
            }
        }
    }

    /// The interval from every held pair, then the prediction.
    fn rebuild(&mut self) {
        // One change of `m` units allows rates in (m - 1, m + 1).
        let moved = self.positions[1] - self.positions[0];
        self.low = Ratio::integer(moved - 1);
        self.high = Ratio::integer(moved + 1);
        for last in 2..=usize::from(self.len) {
            self.narrow(last);
        }
        self.predict();
    }

    /// The phase bounds (times q) position `j` allows at rate `p / q`:
    /// X_j <= phase + rate * j < X_j + 1.
    fn phases(&self, p: i64, q: i64, j: usize) -> [i64; 2] {
        let (at, offset) = (self.positions[j], p * j as i64);
        [at * q - offset, (at + 1) * q - offset]
    }

    /// The prediction from the whole window; none before it holds
    /// [`EVIDENCE`] nonzero changes, whose rate interval is not kept.
    fn predict(&mut self) {
        self.rated = None;
        self.next = 0;
        if usize::from(self.nonzero) < EVIDENCE || !self.feasible() {
            return;
        }
        let rate = simplest(self.low, Some(self.high));
        let phase = (0..=usize::from(self.len)).fold([i64::MIN, i64::MAX], |[low, high], j| {
            let [at_low, at_high] = self.phases(rate.num, rate.den, j);
            [low.max(at_low), high.min(at_high)]
        });
        self.rated = Some((rate, phase));
        self.extrapolate();
    }

    /// The prediction after one more held position, from the last hour's
    /// rate when the interval still holds it.
    fn predict_next(&mut self) {
        match self.rated {
            Some((rate, [low, high]))
                if self.feasible()
                    && self.low.compare(rate) == Ordering::Less
                    && rate.compare(self.high) == Ordering::Less =>
            {
                let [at_low, at_high] = self.phases(rate.num, rate.den, usize::from(self.len));
                self.rated = Some((rate, [low.max(at_low), high.min(at_high)]));
                self.extrapolate();
            }
            _ => self.predict(),
        }
    }

    /// The change to the position at the middle phase one hour after the
    /// last held change.
    fn extrapolate(&mut self) {
        let Some((rate, [low, high])) = self.rated else {
            return;
        };
        let hour = i64::from(self.len) + 1;
        let position = (low + high + 2 * rate.num * hour).div_euclid(2 * rate.den);
        // At most 2^32 units of at most 2^31.
        self.next = (position - self.positions[usize::from(self.len)]) * self.unit;
    }

    /// The ramp after the field changed by `change` (zero when unchanged);
    /// None when it is forgotten.
    pub(super) fn push(self, change: i128) -> Option<Self> {
        let change = i32::try_from(change).ok()?;
        match self.append(change) {
            Appended::Kept(ramp) => Some(ramp),
            Appended::Forgotten => None,
            Appended::Broken => match (self.broken, i32::try_from(self.next)) {
                // A second change no rate allows: a new rate.
                (Some(previous), _) => {
                    match Self::start(i128::from(previous)).map(|ramp| ramp.append(change)) {
                        Some(Appended::Kept(ramp)) => Some(ramp),
                        _ => Self::start(i128::from(change)),
                    }
                }
                // A one-off: the hour as predicted.
                (None, Ok(next)) if next != 0 => match self.append(next) {
                    Appended::Kept(mut ramp) => {
                        ramp.broken = Some(change);
                        Some(ramp)
                    }
                    _ => Self::start(i128::from(change)),
                },
                _ => Self::start(i128::from(change)),
            },
        }
    }

    fn append(mut self, change: i32) -> Appended {
        if change == 0 {
            self.idle += 1;
            if self.idle >= IDLE {
                return Appended::Forgotten;
            }
        } else {
            self.idle = 0;
        }
        // The interval is kept from the hour the window holds its evidence.
        let mut rebuild = usize::from(self.nonzero) < EVIDENCE;
        if usize::from(self.len) == WINDOW {
            let base = self.positions[WINDOW - KEEP];
            for at in 0..=KEEP {
                self.positions[at] = self.positions[WINDOW - KEEP + at] - base;
            }
            self.len = KEEP as u8;
            self.nonzero = self
                .held()
                .windows(2)
                .filter(|pair| pair[0] != pair[1])
                .count() as u8;
            rebuild = true;
        }
        let change = i64::from(change);
        if change % self.unit != 0 {
            // Both at most 2^31, so their divisor is too, and it divides the
            // old unit.
            let unit = gcd(self.unit.unsigned_abs(), change.unsigned_abs()) as i64;
            let scale = self.unit / unit;
            for position in &mut self.positions[..=usize::from(self.len)] {
                *position *= scale;
            }
            self.unit = unit;
            rebuild = true;
        }
        let len = usize::from(self.len);
        self.positions[len + 1] = self.positions[len] + change / self.unit;
        self.len += 1;
        self.nonzero += u8::from(change != 0);
        if self.nonzero == 0 {
            // A kept window of unchanged hours has nothing to move by.
            return Appended::Forgotten;
        }
        if usize::from(self.nonzero) < EVIDENCE {
            (self.next, self.rated) = (0, None);
        } else if rebuild {
            self.rebuild();
        } else {
            self.narrow(usize::from(self.len));
            self.predict_next();
        }
        if !self.feasible() {
            return Appended::Broken;
        }
        self.broken = None;
        Appended::Kept(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The prediction a field moving by `changes` meets before each change.
    fn predictions(changes: &[i128]) -> Vec<i128> {
        let mut ramp: Option<Ramp> = None;
        changes
            .iter()
            .map(|&change| {
                let predicted = ramp.map_or(0, |ramp| ramp.next());
                ramp = match ramp {
                    Some(ramp) => ramp.push(change),
                    None => Ramp::start(change),
                };
                predicted
            })
            .collect()
    }

    fn misses(changes: &[i128]) -> usize {
        predictions(changes)
            .iter()
            .zip(changes)
            .filter(|(predicted, change)| predicted != change)
            .count()
    }

    #[test]
    fn simplest_takes_the_smallest_denominator_strictly_inside() {
        let r = Ratio::new;
        assert_eq!(simplest(r(-2, 1), Some(r(0, 1))), r(-1, 1));
        assert_eq!(simplest(r(-8, 1), Some(r(-7, 1))), r(-15, 2));
        assert_eq!(simplest(r(-1, 2), Some(r(0, 1))), r(-1, 3));
        assert_eq!(simplest(r(-1, 4), Some(r(0, 1))), r(-1, 5));
        assert_eq!(simplest(r(4, 3), Some(r(5, 3))), r(3, 2));
        assert_eq!(simplest(r(13, 10), Some(r(4, 3))), r(17, 13));
        assert_eq!(simplest(r(0, 1), None), r(1, 1));
        // Open: an endpoint is never the answer.
        assert_eq!(simplest(r(1, 1), Some(r(2, 1))), r(3, 2));
    }

    #[test]
    fn constant_alternating_beatty_and_sparse_changes_are_predicted() {
        // A constant change from its third hour.
        assert_eq!(misses(&[5; 12]), 2);
        // -8, -7: falling 7.5 an hour.
        let alternating: Vec<i128> = (0..40).map(|t| if t % 2 == 0 { -8 } else { -7 }).collect();
        assert!(misses(&alternating) <= 3, "{:?}", predictions(&alternating));
        // floor(0.3 + 1.27 t): a long period.
        let beatty: Vec<i128> = (1..=60)
            .map(|t: i128| (30 + 127 * t) / 100 - (30 + 127 * (t - 1)) / 100)
            .collect();
        assert!(misses(&beatty) <= 12, "{:?}", predictions(&beatty));
        // One percent of a 72-hour grace every fifth hour.
        let sparse: Vec<i128> = (0..60).map(|t| if t % 5 == 4 { -72 } else { 0 }).collect();
        assert!(misses(&sparse) <= 8, "{:?}", predictions(&sparse));
    }

    #[test]
    fn one_off_changes_predict_nothing_and_idle_fields_are_forgotten() {
        let mut changes = vec![0; 20];
        changes[3] = -1;
        changes[14] = 2;
        assert_eq!(predictions(&changes), vec![0; 20]);
        let mut ramp = Ramp::start(3);
        for _ in 1..IDLE {
            ramp = ramp.and_then(|ramp| ramp.push(0));
            assert!(ramp.is_some());
        }
        assert_eq!(ramp.and_then(|ramp| ramp.push(0)), None);
        assert_eq!(Ramp::start(0), None);
        assert_eq!(Ramp::start(i128::from(i32::MAX) + 1), None);
        let lowest = Ramp::start(i128::from(i32::MIN)).unwrap();
        assert_eq!((lowest.held(), lowest.unit), (&[0, -1][..], 1 << 31));
        assert_eq!(Ramp::start(5).unwrap().push(i128::MIN), None);
    }

    #[test]
    fn a_kept_rate_extends_its_phases_as_the_whole_window_would() {
        // Organisation at 0.127 an hour, acclimatization at -7.5 and a mix.
        for changes in [
            (1..=200)
                .map(|t: i64| (30 + 127 * t) / 100 - (30 + 127 * (t - 1)) / 100)
                .collect::<Vec<_>>(),
            (0..80).map(|t| if t % 2 == 0 { -8 } else { -7 }).collect(),
            (0..80).map(|t| [3, 0, 4, 3, 0, -50, 3, 4][t % 8]).collect(),
        ] {
            // Every window is compared, those still short of their evidence
            // (a start, a restart after a broken rate) included.
            let mut ramp = Ramp::start(i128::from(changes[0]));
            for change in &changes[1..] {
                ramp = ramp.and_then(|ramp| ramp.push(i128::from(*change)));
                if let Some(ramp) = ramp {
                    let mut whole = ramp;
                    whole.predict();
                    assert_eq!((whole.next, whole.rated), (ramp.next, ramp.rated));
                }
            }
        }
        // A window short of its evidence keeps no rate interval, so the
        // whole window predicts nothing either.
        let mut short = Ramp::start(3).unwrap().push(0).unwrap();
        assert_eq!(usize::from(short.nonzero), EVIDENCE - 1);
        short.predict();
        assert_eq!((short.next, short.rated), (0, None));
    }

    #[test]
    fn a_change_no_rate_allows_starts_the_window_again() {
        let mut ramp = Ramp::start(10).unwrap();
        for _ in 0..5 {
            ramp = ramp.push(10).unwrap();
        }
        assert_eq!(ramp.next(), 10);
        // A one-off keeps the rate; a second change no rate allows starts
        // again at both.
        ramp = ramp.push(-400).unwrap();
        assert_eq!(ramp.next(), 10);
        ramp = ramp.push(-400).unwrap();
        assert_eq!((ramp.held(), ramp.unit), (&[0, -1, -2][..], 400));
        assert_eq!(ramp.next(), -400);
        // A wrapping clock costs one correction a wrap once learned.
        let clock: Vec<i128> = (1..=72)
            .map(|hour| if hour % 24 == 0 { -23 } else { 1 })
            .collect();
        let missed: Vec<usize> = predictions(&clock)
            .iter()
            .zip(&clock)
            .enumerate()
            .filter(|(_, (predicted, change))| predicted != change)
            .map(|(hour, _)| hour + 1)
            .collect();
        assert_eq!(missed, [1, 2, 24, 48, 72]);
        // The extreme changes keep exact arithmetic.
        let mut ramp = Ramp::start(i128::from(i32::MAX)).unwrap();
        for change in [i32::MAX, i32::MIN + 1, i32::MAX, i32::MAX] {
            ramp = ramp.push(i128::from(change)).unwrap();
        }
        assert_eq!(ramp.next(), i128::from(i32::MAX));
        // A full window keeps its latest changes.
        let mut ramp = Ramp::start(1).unwrap();
        for _ in 1..40 {
            ramp = ramp.push(1).unwrap();
            assert!(usize::from(ramp.len) <= WINDOW);
            assert_eq!(ramp.next(), 1);
        }
    }
}
