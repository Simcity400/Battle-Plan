//! Cipher races: every pair at work on a cipher at an hour ([`Decryption`], rows at an hour),
//! each with when its current attempt started and broke. Those come from the timeline's cipher
//! history ([`CipherHistory`]: one history of Cryptology and one of RaidIntelInputs' strength a
//! stretch, read once and kept), folded by the game's start and break rules for the content's
//! thresholds ([`Attempts`], kept per thresholds). Progress and the bonus countdown move daily:
//! they are live cells, never in the reply.

use super::{Content, Country, IDENTITY, Tags, branch, missing};
use crate::{
    Source,
    products::{
        LoadContext,
        at_hour::{At, AtHour},
    },
};
use dashboard_follow::live::Source as Cell;
use dashboard_query::{Index, Query, timeline::Join};
use dashboard_text::cryptology;
use replay::{campaign::Replay, lanes, points::Cell as Read};
use replay_data::rows::{CryptologyRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, Mutex, PoisonError},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const TABLES: [TableId; 4] = [TableId::Cryptology, TableId::RaidIntelInputs, IDENTITY[0], IDENTITY[1]];

/// Cryptology's columns, as the schema numbers them.
const AMOUNT: usize = 0;
const BONUS_DAYS: usize = 1;
const BONUS_ACTIVE: usize = 2;
const DECRYPTING: usize = 3;
/// RaidIntelInputs' cryptology strength.
const STRENGTH: usize = 2;

/// A pair's Cryptology key: its owner's raw id over its target's.
const fn pair_key(owner: u32, target: u32) -> u64 {
    ((owner as u64) << 32) | target as u64
}

pub struct Decryption;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecryptionParams {
    pub branch_id: String,
}

/// One pair's cipher at the hour. Its amount (`amount`) and bonus countdown (`bonusDays`) are live
/// cells of the row `<owner id>:<target id>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CipherPair {
    pub owner: Country,
    pub target: Country,
    /// Fixed5: the amount that breaks the cipher; none where it is not known.
    pub threshold_raw: Option<String>,
    /// Why the threshold is not known, where it is not.
    pub threshold_unknown: Option<String>,
    pub bonus_active: u8,
    pub decrypting: u8,
    /// None where the threshold is not known and no bonus is active.
    pub broken: Option<bool>,
    /// When the attempt current at the hour started; none when none is, or it is not known.
    pub started: Option<Observation>,
    /// When that attempt broke the cipher; none when it has not.
    pub broken_at: Option<Observation>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DecryptionReply {
    /// The pairs with an attempt or a broken cipher, in the game's race order: broken first, then
    /// by progress (unknown last), then by decryptor and target.
    pub pairs: Vec<CipherPair>,
    /// Why the hour's ciphers are not known, when they are not.
    pub missing: Option<String>,
    /// Why the attempts' starts and breaks are not known, when they are not: every pair at work
    /// is then listed with neither.
    pub attempts_unknown: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(DecryptionReply);

/// What a target's threshold is at the hour.
fn threshold(strength: Option<&Option<i64>>, defines: &Result<cryptology::Defines, String>) -> Result<i64, String> {
    let defines = defines.as_ref().map_err(|reason| format!("Cipher defines unknown: {reason}"))?;
    match strength {
        Some(Some(strength)) => Ok(cryptology::threshold(*strength, *defines)),
        Some(None) => Err("raid intel inputs holds no cryptology strength for the target".into()),
        None => Err("The target's raid intel inputs row is not recorded at this hour".into()),
    }
}

/// A pair at the hour beside the numbers its place in the race is read from.
struct Ranked {
    pair: CipherPair,
    amount: i64,
    required: Option<i64>,
    percent: Option<u8>,
}

/// The game's race order: broken first, unknown progress last, more progress first, then the exact
/// share of the threshold (a positive threshold before one that is not), then decryptor and target
/// by tag (a country no tag names after), then by id.
fn race_order(a: &Ranked, b: &Ranked) -> Ordering {
    let broken = |item: &Ranked| item.pair.broken == Some(true);
    let tag = |country: &Country| (country.tag.is_none(), country.tag.clone(), country.id);
    broken(b)
        .cmp(&broken(a))
        .then_with(|| a.percent.is_none().cmp(&b.percent.is_none()))
        .then_with(|| b.percent.cmp(&a.percent))
        .then_with(|| match (a.required, b.required) {
            (Some(x), Some(y)) if x > 0 && y > 0 => (i128::from(b.amount) * i128::from(x)).cmp(&(i128::from(a.amount) * i128::from(y))),
            (Some(x), Some(y)) => (y > 0).cmp(&(x > 0)),
            _ => Ordering::Equal,
        })
        .then_with(|| tag(&a.pair.owner).cmp(&tag(&b.pair.owner)))
        .then_with(|| tag(&a.pair.target).cmp(&tag(&b.pair.target)))
}

/// The hour's pairs with an attempt or a broken cipher (with `attempts` unknown, those at work, broken
/// or holding progress), each with its attempt from `attempts`, in race order; each pair's amount and
/// bonus countdown named as live cells in `at`.
fn pairs(
    frame: &DecodedFrame,
    defines: &Result<cryptology::Defines, String>,
    attempts: Option<&Attempts>,
    live: &dashboard_follow::live::Cells,
) -> io::Result<Vec<CipherPair>> {
    let tags = Tags::read(frame)?;
    let rows = frame.complete::<tables::Cryptology>()?;
    let strengths: Option<HashMap<u32, Option<i64>>> =
        frame.complete::<tables::RaidIntelInputs>().ok().map(|rows| rows.iter().map(|row| (row.country, row.crypto_strength)).collect());
    let mut out: Vec<Ranked> = Vec::new();
    for row in rows.iter() {
        let threshold = match &strengths {
            Some(strengths) => threshold(strengths.get(&row.target_country), defines),
            None => Err(missing(frame, TableId::RaidIntelInputs).unwrap_or_else(|| "raid intel inputs is not recorded".into())),
        };
        let required = threshold.as_ref().ok().copied();
        let broken = cryptology::fully_decrypted(row.amount_raw, row.bonus_active, required);
        let key = pair_key(row.owner_country, row.target_country);
        let (started, broken_at) = match attempts {
            Some(attempts) => attempts.at(key, frame.hour),
            None => (None, None),
        };
        let shown = match attempts {
            Some(_) => started.is_some() || broken == Some(true),
            None => row.decrypting != 0 || row.bonus_active != 0 || row.amount_raw > 0,
        };
        if !shown {
            continue;
        }
        name_cells(live, row);
        out.push(Ranked {
            amount: row.amount_raw,
            required,
            percent: required.and_then(|required| cryptology::progress_percent(row.amount_raw, row.bonus_active, required)),
            pair: CipherPair {
                owner: Country::of(&tags, row.owner_country),
                target: Country::of(&tags, row.target_country),
                threshold_raw: required.map(|value| value.to_string()),
                threshold_unknown: threshold.err(),
                bonus_active: row.bonus_active,
                decrypting: row.decrypting,
                broken,
                started,
                broken_at,
            },
        });
    }
    out.sort_by(race_order);
    Ok(out.into_iter().map(|ranked| ranked.pair).collect())
}

/// A pair's amount and bonus countdown, as the live cells of its row.
fn name_cells(live: &dashboard_follow::live::Cells, row: &CryptologyRow) {
    let id = format!("{}:{}", row.owner_country, row.target_country);
    let key = u128::from(pair_key(row.owner_country, row.target_country));
    live.value(&id, "amount", Cell { table: TableId::Cryptology, column: AMOUNT, key }, 1.0);
    live.value(&id, "bonusDays", Cell { table: TableId::Cryptology, column: BONUS_DAYS, key }, 1.0);
}

/// What a reply reads beside its frame: the content's thresholds and the timelines' attempts.
pub struct DecryptionContext {
    content: Content,
    ciphers: Ciphers,
}

impl AtHour for Decryption {
    const KEY: &'static str = "decryption";
    const TITLE: &'static str = "Decryption";
    const CONTENT_DEPENDENT: bool = true;
    type Params = DecryptionParams;
    type Reply = DecryptionReply;
    type Context = DecryptionContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![DecryptionParams::decl(), Country::decl(), Observation::decl(), CipherPair::decl(), DecryptionReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(DecryptionContext { content: Content::load(source, load), ciphers: Ciphers::of(source) })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut warnings = Vec::new();
        let defines = &at.context.content.defines;
        if let Err(reason) = defines {
            warnings.push(format!("Cipher thresholds unknown, so a cipher is known broken only by its bonus: {reason}"));
        }
        let branch: u64 = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let attempts = at.context.ciphers.attempts(branch, defines.as_ref().ok().copied());
        let attempts_unknown = attempts.as_ref().err().map(|error| format!("Attempt starts and breaks are not known: {error}"));
        Ok(match pairs(frame, defines, attempts.as_deref().ok(), at.live) {
            Ok(pairs) => DecryptionReply { pairs, missing: None, attempts_unknown, warnings },
            Err(error) => DecryptionReply { pairs: Vec::new(), missing: Some(error.to_string()), attempts_unknown, warnings },
        })
    }
}

/// One cipher's state through some hours: what the game's start and break rules read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CipherState {
    pub amount: i64,
    pub decrypting: bool,
    pub bonus_active: bool,
    /// None where the threshold is not known and no bonus is active.
    pub broken: Option<bool>,
}

impl CipherState {
    fn works(&self) -> bool {
        self.decrypting || self.amount > 0
    }
}

/// An hour something was seen: exact, or by that hour at the latest (already so at the first hour
/// read after hours not known).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub hour: i32,
    pub upper_bound: bool,
}

/// The current attempt's start and break, fed one stretch of hours of one state at a time, in hour
/// order, as the game's rules read them: starting sets work; a bonus ending, or work dropping its
/// progress to nothing, ends an attempt. A state none (the pair not known) ends continuity, as do
/// hours apart and [`Fold::reset`].
#[derive(Default)]
struct Fold {
    started: Option<Observation>,
    broke: Option<Observation>,
    previous: Option<CipherState>,
    idle_known: bool,
    last_end: Option<i32>,
}

impl Fold {
    /// Nothing carries from before: a break in the timeline.
    fn reset(&mut self) {
        self.previous = None;
        self.idle_known = false;
    }

    /// `first` through `last` in `state`; the attempt current after them.
    fn step(&mut self, first: i32, last: i32, state: Option<CipherState>) -> (Option<Observation>, Option<Observation>) {
        let contiguous = self.last_end.is_some_and(|end| end + 1 == first);
        self.last_end = Some(last);
        let Some(now) = state else {
            self.reset();
            return (self.started, self.broke);
        };
        match self.previous.filter(|_| contiguous) {
            None => {
                self.idle_known = false;
                self.started = (now.works() || now.bonus_active).then_some(Observation { hour: first, upper_bound: true });
                self.broke = (now.broken == Some(true) && self.started.is_some()).then_some(Observation { hour: first, upper_bound: true });
            }
            Some(before) => {
                let ended = (before.bonus_active && !now.bonus_active) || (before.amount > 0 && now.amount <= 0 && !now.decrypting && !now.bonus_active);
                if ended {
                    self.idle_known = true;
                    self.started = None;
                    self.broke = None;
                }
                if self.started.is_none() && (now.decrypting || now.bonus_active || (now.amount > 0 && now.amount > before.amount)) {
                    self.started = Some(Observation { hour: first, upper_bound: !self.idle_known });
                }
                if self.started.is_some() && self.broke.is_none() && now.broken == Some(true) {
                    self.broke = Some(Observation { hour: first, upper_bound: before.broken.is_none() });
                }
            }
        }
        self.previous = Some(now);
        (self.started, self.broke)
    }
}

/// The current attempt's start and break through `states` (hour order; a state none where the
/// pair was not known, which ends any continuity).
pub fn attempt(states: &[(i32, i32, Option<CipherState>)]) -> (Option<Observation>, Option<Observation>) {
    let mut fold = Fold::default();
    let mut current = (None, None);
    for &(first, last, state) in states {
        current = fold.step(first, last, state);
    }
    current
}

const KNOWN: u8 = 1;
const WORKING: u8 = 2;
const BONUS: u8 = 4;
/// Nothing carries from before this step: the timeline breaks here.
const FRESH: u8 = 8;

/// One pair's Cryptology row through a timeline, from each hour it changed: its amount and flags
/// (not [`KNOWN`] where the row was not read).
#[derive(Default)]
struct PairSteps {
    hours: Vec<i32>,
    amounts: Vec<i64>,
    flags: Vec<u8>,
}

impl PairSteps {
    /// From `hour` on, `amount` and `flags`: a step at the same hour is replaced (keeping its
    /// [`FRESH`]), one equal to the step before is not kept.
    fn push(&mut self, hour: i32, amount: i64, mut flags: u8) {
        if self.hours.last() == Some(&hour) {
            self.hours.pop();
            self.amounts.pop();
            flags |= self.flags.pop().unwrap_or(0) & FRESH;
        }
        if flags & FRESH == 0 && self.flags.last().is_some_and(|&last| last & !FRESH == flags) && (flags & KNOWN == 0 || self.amounts.last() == Some(&amount)) {
            return;
        }
        self.hours.push(hour);
        self.amounts.push(if flags & KNOWN == 0 { 0 } else { amount });
        self.flags.push(flags);
    }
}

/// From `hour` on, a target's strength (none where not known), replacing a step at the same hour
/// and keeping none equal to the one before.
fn push_strength(steps: &mut Vec<(i32, Option<i64>)>, hour: i32, strength: Option<i64>) {
    if steps.last().is_some_and(|last| last.0 == hour) {
        steps.pop();
    }
    if steps.last().is_none_or(|last| last.1 != strength) {
        steps.push((hour, strength));
    }
}

fn number(cell: Read) -> Option<i64> {
    match cell {
        Read::Number(value) => Some(value),
        _ => None,
    }
}

/// A timeline's ciphers: each pair's Cryptology steps and each target's cryptology strength, read
/// once (one history of each table a stretch, at the cost of their changes).
pub struct CipherHistory {
    /// The timeline's last hour: each last step holds through it.
    last: i32,
    pairs: BTreeMap<u64, PairSteps>,
    strengths: HashMap<u32, Vec<(i32, Option<i64>)>>,
}

impl CipherHistory {
    fn read(query: &Query<'_>) -> io::Result<Self> {
        let recording = query.replay.recording();
        let mut out = Self { last: query.timeline.last, pairs: BTreeMap::new(), strengths: HashMap::new() };
        for stretch in &query.timeline.stretches {
            let fresh = if matches!(stretch.join, Join::Load { .. }) { 0 } else { FRESH };
            // A row this stretch's reads do not show at its start is not known there.
            for steps in out.pairs.values_mut() {
                steps.push(stretch.first, 0, fresh);
            }
            for steps in out.strengths.values_mut() {
                push_strength(steps, stretch.first, None);
            }
            let ciphers = dashboard_query::cost::storage(|| {
                lanes::history(recording, stretch.run, TableId::Cryptology, &[AMOUNT, BONUS_ACTIVE, DECRYPTING], lanes::Keys::All, stretch.first, stretch.last)
            })?;
            for (key, steps) in &ciphers.keys {
                let pair = out.pairs.entry(*key as u64).or_default();
                if pair.hours.is_empty() {
                    pair.push(stretch.first, 0, fresh);
                }
                for step in steps {
                    let cells = step.cells.as_deref().map(|cells| (number(cells[0]), number(cells[1]), number(cells[2])));
                    match cells {
                        Some((Some(amount), Some(bonus), Some(working))) => {
                            let flags = KNOWN | if working != 0 { WORKING } else { 0 } | if bonus != 0 { BONUS } else { 0 };
                            pair.push(step.hour, amount, flags);
                        }
                        _ => pair.push(step.hour, 0, 0),
                    }
                }
            }
            let inputs = dashboard_query::cost::storage(|| {
                lanes::history(recording, stretch.run, TableId::RaidIntelInputs, &[STRENGTH], lanes::Keys::All, stretch.first, stretch.last)
            })?;
            for (key, steps) in &inputs.keys {
                let strengths = out.strengths.entry(*key as u32).or_default();
                for step in steps {
                    push_strength(strengths, step.hour, step.cells.as_deref().and_then(|cells| number(cells[0])));
                }
            }
        }
        Ok(out)
    }

    /// Each pair's attempts with `defines`' thresholds (none: a cipher breaks only by its bonus).
    fn attempts(&self, defines: Option<cryptology::Defines>) -> Attempts {
        let none = Vec::new();
        let mut out = HashMap::with_capacity(self.pairs.len());
        for (&key, steps) in &self.pairs {
            let Some(&first) = steps.hours.first() else { continue };
            let strengths = self.strengths.get(&(key as u32)).unwrap_or(&none);
            let mut fold = Fold::default();
            let mut made: Vec<AttemptStep> = Vec::new();
            let (mut at, mut seen) = (0, 0);
            let mut hour = first;
            loop {
                while at + 1 < steps.hours.len() && steps.hours[at + 1] <= hour {
                    at += 1;
                }
                while seen < strengths.len() && strengths[seen].0 <= hour {
                    seen += 1;
                }
                let next = match (steps.hours.get(at + 1), strengths.get(seen)) {
                    (Some(&pair), Some(&(strength, _))) => Some(pair.min(strength)),
                    (Some(&pair), None) => Some(pair),
                    (None, Some(&(strength, _))) => Some(strength),
                    (None, None) => None,
                };
                let last = next.map_or(self.last.max(hour), |next| next - 1);
                let flags = steps.flags[at];
                if steps.hours[at] == hour && flags & FRESH != 0 {
                    fold.reset();
                }
                let state = (flags & KNOWN != 0).then(|| {
                    let required = seen.checked_sub(1).and_then(|step| strengths[step].1).and_then(|strength| defines.map(|defines| cryptology::threshold(strength, defines)));
                    let active = u8::from(flags & BONUS != 0);
                    CipherState {
                        amount: steps.amounts[at],
                        decrypting: flags & WORKING != 0,
                        bonus_active: active != 0,
                        broken: cryptology::fully_decrypted(steps.amounts[at], active, required),
                    }
                });
                let (started, broke) = fold.step(hour, last, state);
                if made.last().is_none_or(|step| (step.started, step.broke) != (started, broke)) {
                    made.push(AttemptStep { hour, started, broke });
                }
                match next {
                    Some(next) => hour = next,
                    None => break,
                }
            }
            out.insert(key, made);
        }
        Attempts(out)
    }
}

/// From `hour` on, a pair's current attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AttemptStep {
    hour: i32,
    started: Option<Observation>,
    broke: Option<Observation>,
}

/// Each pair's attempts through a timeline, by its key.
pub struct Attempts(HashMap<u64, Vec<AttemptStep>>);

impl Attempts {
    /// The start and break of pair `key`'s attempt current at `hour`.
    fn at(&self, key: u64, hour: i32) -> (Option<Observation>, Option<Observation>) {
        self.0
            .get(&key)
            .and_then(|steps| steps.get(steps.partition_point(|step| step.hour <= hour).checked_sub(1)?))
            .map_or((None, None), |step| (step.started, step.broke))
    }
}

#[derive(Default)]
struct Held {
    histories: HashMap<u64, Arc<CipherHistory>>,
    attempts: HashMap<(u64, Option<(i64, i64)>), Arc<Attempts>>,
}

/// A recording's cipher histories and attempts, by timeline: each read once while it is open.
pub struct Ciphers {
    replay: Arc<Replay>,
    index: Arc<Index>,
    held: Arc<Mutex<Held>>,
}

impl Ciphers {
    pub fn of(source: &Source) -> Self {
        Self { replay: source.replay(), index: crate::answer::index(source), held: source.state::<Held>() }
    }

    fn history(&self, branch: u64) -> io::Result<Arc<CipherHistory>> {
        if let Some(found) = self.held.lock().unwrap_or_else(PoisonError::into_inner).histories.get(&branch) {
            return Ok(Arc::clone(found));
        }
        let made = Arc::new(CipherHistory::read(&self.index.query(&self.replay, branch)?)?);
        self.held.lock().unwrap_or_else(PoisonError::into_inner).histories.insert(branch, Arc::clone(&made));
        Ok(made)
    }

    /// Timeline `branch`'s attempts with `defines`' thresholds.
    fn attempts(&self, branch: u64, defines: Option<cryptology::Defines>) -> io::Result<Arc<Attempts>> {
        let key = (branch, defines.map(|defines| (defines.base, defines.upgrade)));
        if let Some(found) = self.held.lock().unwrap_or_else(PoisonError::into_inner).attempts.get(&key) {
            return Ok(Arc::clone(found));
        }
        let made = Arc::new(self.history(branch)?.attempts(defines));
        self.held.lock().unwrap_or_else(PoisonError::into_inner).attempts.insert(key, Arc::clone(&made));
        Ok(made)
    }
}

/// Reads timeline `branch`'s cipher history, as opening the recording does.
pub fn open(source: &Source, branch: u64) -> io::Result<()> {
    Ciphers::of(source).history(branch).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFINES: cryptology::Defines = cryptology::Defines { base: 0, upgrade: 100_000 };

    fn steps(entries: &[(i32, i64, u8)]) -> PairSteps {
        let mut out = PairSteps::default();
        for &(hour, amount, flags) in entries {
            out.push(hour, amount, flags);
        }
        out
    }

    fn history(pairs: Vec<(u64, PairSteps)>, strengths: Vec<(u32, Vec<(i32, Option<i64>)>)>, last: i32) -> CipherHistory {
        CipherHistory { last, pairs: pairs.into_iter().collect(), strengths: strengths.into_iter().collect() }
    }

    const fn seen(hour: i32, upper_bound: bool) -> Option<Observation> {
        Some(Observation { hour, upper_bound })
    }

    #[test]
    fn a_pairs_attempt_at_an_hour_is_the_fold_of_its_steps_with_its_targets_threshold_as_it_stood() {
        let key = pair_key(1, 2);
        // Idle, then at work from 20; the target's strength rises at 40, after the break.
        let pair = steps(&[(0, 0, KNOWN | FRESH), (20, 0, KNOWN | WORKING), (30, 300_000, KNOWN | WORKING), (60, 900_000, KNOWN | WORKING)]);
        let strength = vec![(0, Some(300_000)), (40, Some(800_000))];
        let attempts = history(vec![(key, pair)], vec![(2, strength)], 99).attempts(Some(DEFINES));
        assert_eq!(attempts.at(key, 10), (None, None));
        assert_eq!(attempts.at(key, 25), (seen(20, true), None));
        // At 30 the amount meets the first threshold: broken then, and still broken after the rise.
        assert_eq!(attempts.at(key, 35), (seen(20, true), seen(30, false)));
        assert_eq!(attempts.at(key, 99), (seen(20, true), seen(30, false)));
        // Without the defines no threshold is known: no break is dated.
        let unknown = history(vec![(key, steps(&[(0, 0, KNOWN | FRESH), (20, 0, KNOWN | WORKING), (30, 300_000, KNOWN | WORKING)]))], vec![], 99).attempts(None);
        assert_eq!(unknown.at(key, 50), (seen(20, true), None));
        assert_eq!(attempts.at(pair_key(9, 9), 50), (None, None));
    }

    #[test]
    fn a_break_in_the_timeline_ends_continuity_so_the_attempt_after_it_is_dated_by_its_first_hour() {
        let key = pair_key(1, 2);
        let pair = steps(&[(0, 0, KNOWN | FRESH), (10, 100, KNOWN | WORKING), (50, 100, KNOWN | WORKING | FRESH)]);
        let attempts = history(vec![(key, pair)], vec![], 99).attempts(Some(DEFINES));
        assert_eq!(attempts.at(key, 20), (seen(10, true), None));
        assert_eq!(attempts.at(key, 60), (seen(50, true), None));
        // A row not read ends continuity too.
        let gap = steps(&[(0, 100, KNOWN | WORKING | FRESH), (10, 0, 0), (20, 200, KNOWN | WORKING)]);
        let attempts = history(vec![(key, gap)], vec![], 99).attempts(Some(DEFINES));
        assert_eq!(attempts.at(key, 5), (seen(0, true), None));
        assert_eq!(attempts.at(key, 15), (seen(0, true), None));
        assert_eq!(attempts.at(key, 25), (seen(20, true), None));
    }

    #[test]
    fn a_step_equal_to_the_one_before_is_not_kept_unless_the_timeline_breaks_there() {
        let pair = steps(&[(0, 5, KNOWN), (10, 5, KNOWN), (20, 5, KNOWN | FRESH), (20, 7, KNOWN), (30, 9, 0), (40, 3, 0)]);
        assert_eq!(pair.hours, vec![0, 20, 30]);
        assert_eq!(pair.flags, vec![KNOWN, KNOWN | FRESH, 0]);
        assert_eq!(pair.amounts, vec![5, 7, 0]);
    }

    fn ranked(owner: &str, target: &str, amount: i64, required: Option<i64>, broken: Option<bool>) -> Ranked {
        let country = |tag: &str, id| Country { id, tag: Some(tag.to_owned()) };
        Ranked {
            amount,
            required,
            percent: required.and_then(|required| cryptology::progress_percent(amount, 0, required)),
            pair: CipherPair {
                owner: country(owner, 1),
                target: country(target, 2),
                threshold_raw: required.map(|value| value.to_string()),
                threshold_unknown: required.is_none().then(|| "unknown".to_owned()),
                bonus_active: 0,
                decrypting: 1,
                broken,
                started: None,
                broken_at: None,
            },
        }
    }

    #[test]
    fn the_race_puts_broken_ciphers_first_then_progress_then_the_exact_share_and_unknown_progress_last() {
        let mut race = vec![
            ranked("GER", "ENG", 400, Some(1_000), Some(false)),
            ranked("ENG", "GER", 100, None, None),
            ranked("SOV", "GER", 900, Some(1_000), Some(false)),
            ranked("ITA", "FRA", 1_000, Some(1_000), Some(true)),
            // 40% as GER's, but a larger exact share of its threshold.
            ranked("JAP", "USA", 4_049, Some(10_000), Some(false)),
        ];
        race.sort_by(race_order);
        let owners: Vec<&str> = race.iter().map(|item| item.pair.owner.tag.as_deref().unwrap()).collect();
        assert_eq!(owners, ["ITA", "SOV", "JAP", "GER", "ENG"]);
    }

    #[test]
    fn the_schema_numbers_the_cipher_columns() {
        for (name, column) in [("amount_raw", AMOUNT), ("bonus_days", BONUS_DAYS), ("bonus_active", BONUS_ACTIVE), ("decrypting", DECRYPTING)] {
            assert_eq!(dashboard_declare::field(TableId::Cryptology, name).unwrap(), column, "{name}");
        }
        assert_eq!(dashboard_declare::field(TableId::RaidIntelInputs, "crypto_strength").unwrap(), STRENGTH);
    }
}
