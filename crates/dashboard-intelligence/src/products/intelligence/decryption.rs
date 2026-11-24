
use super::{Content, Country, IDENTITY, Tags, branch, missing};
use dashboard_base::{Source, products::{LoadContext, at_hour::{At, AtHour}}};
use dashboard_follow::live::Source as Cell;
use dashboard_query::{Index, Query, timeline::Join};
use dashboard_text::cryptology;
use replay::campaign::Replay;
use replay_lanes::{lanes, points::Cell as Read};
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

const AMOUNT: usize = 0;
const BONUS_DAYS: usize = 1;
const BONUS_ACTIVE: usize = 2;
const DECRYPTING: usize = 3;
const STRENGTH: usize = 2;
const WEEK_HOURS: i32 = 168;

const fn pair_key(owner: u32, target: u32) -> u64 {
    ((owner as u64) << 32) | target as u64
}

pub struct Decryption;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecryptionParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CipherPair {
    pub owner: Country,
    pub target: Country,
    #[doc = " Fixed5: the amount that breaks the cipher; none where it is not known."]
    pub threshold_raw: Option<String>,
    #[doc = " Why the threshold is not known, where it is not."]
    pub threshold_unknown: Option<String>,
    pub bonus_active: u8,
    pub decrypting: u8,
    #[doc = " None where the threshold is not known and no bonus is active."]
    pub broken: Option<bool>,
    #[doc = " When the attempt current at the hour started; none when none is, or it is not known."]
    pub started: Option<Observation>,
    #[doc = " When that attempt broke the cipher; none when it has not."]
    pub broken_at: Option<Observation>,
    #[doc = " The change in progress over the last 7 days, in tenths of a percentage point of the threshold,"]
    #[doc = " from the reading 168 hours earlier in the same unbroken stretch; none where it is not known."]
    pub week_change_tenths: Option<i32>,
    #[doc = " Why the change over the last 7 days is not known, where it is not."]
    pub week_change_unknown: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DecryptionReply {
    #[doc = " The pairs with an attempt or a broken cipher, in the game's race order: broken first, then"]
    #[doc = " by progress (unknown last), then by decryptor and target."]
    pub pairs: Vec<CipherPair>,
    #[doc = " Why the hour's ciphers are not known, when they are not."]
    pub missing: Option<String>,
    #[doc = " Why the attempts' starts and breaks are not known, when they are not: every pair at work"]
    #[doc = " is then listed with neither."]
    pub attempts_unknown: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(DecryptionReply);

fn threshold(strength: Option<&Option<i64>>, defines: &Result<cryptology::Defines, String>) -> Result<i64, String> {
    let defines = defines.as_ref().map_err(|reason| format!("Cipher defines unknown: {reason}"))?;
    match strength {
        Some(Some(strength)) => Ok(cryptology::threshold(*strength, *defines)),
        Some(None) => Err("raid intel inputs holds no cryptology strength for the target".into()),
        None => Err("The target's raid intel inputs row is not recorded at this hour".into()),
    }
}

struct Ranked {
    pair: CipherPair,
    amount: i64,
    required: Option<i64>,
    percent: Option<u8>,
}

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

fn week_change(now: i64, then: Result<i64, String>, required: &Result<i64, String>) -> Result<i32, String> {
    let required = match required {
        Ok(required) if *required > 0 => *required,
        Ok(_) => return Err("The cipher's threshold is zero, so its progress has no scale".into()),
        Err(reason) => return Err(format!("Progress is not known: {reason}")),
    };
    let then = then?;
    let scaled = (i128::from(now) - i128::from(then)) * 1_000;
    let required = i128::from(required);
    let half = required / 2;
    let rounded = if scaled >= 0 { (scaled + half) / required } else { (scaled - half) / required };
    i32::try_from(rounded).map_err(|_| "The change over the last 7 days is out of range".into())
}

fn pairs(
    frame: &DecodedFrame,
    defines: &Result<cryptology::Defines, String>,
    attempts: Option<&Attempts>,
    history: Result<&CipherHistory, &str>,
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
        let earlier = match history {
            Ok(history) => history.amount_at(key, frame.hour - WEEK_HOURS, frame.hour),
            Err(reason) => Err(format!("Not recorded 7 days earlier: {reason}")),
        };
        let change = week_change(row.amount_raw, earlier, &threshold);
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
                week_change_tenths: change.as_ref().ok().copied(),
                week_change_unknown: change.err(),
            },
        });
    }
    out.sort_by(race_order);
    Ok(out.into_iter().map(|ranked| ranked.pair).collect())
}

fn name_cells(live: &dashboard_follow::live::Cells, row: &CryptologyRow) {
    let id = format!("{}:{}", row.owner_country, row.target_country);
    let key = u128::from(pair_key(row.owner_country, row.target_country));
    live.value(&id, "amount", Cell { table: TableId::Cryptology, column: AMOUNT, key }, 1.0);
    live.value(&id, "bonusDays", Cell { table: TableId::Cryptology, column: BONUS_DAYS, key }, 1.0);
}

pub struct DecryptionContext {
    content: Content,
    ciphers: Ciphers,
}

impl AtHour for Decryption {
    const KEY: &'static str = "decryption";
    const TITLE: &'static str = "Decryption";
    const CONTENT_DEPENDENT: bool = true;
    type Params = DecryptionParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
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
        let history = at.context.ciphers.history(branch);
        let history_error = history.as_ref().err().map(ToString::to_string).unwrap_or_default();
        let history = history.as_deref().map_err(|_| history_error.as_str());
        Ok(match pairs(frame, defines, attempts.as_deref().ok(), history, at.live) {
            Ok(pairs) => DecryptionReply { pairs, missing: None, attempts_unknown, warnings },
            Err(error) => DecryptionReply { pairs: Vec::new(), missing: Some(error.to_string()), attempts_unknown, warnings },
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CipherState {
    pub amount: i64,
    pub decrypting: bool,
    pub bonus_active: bool,
    pub broken: Option<bool>,
}

impl CipherState {
    fn works(&self) -> bool {
        self.decrypting || self.amount > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub hour: i32,
    pub upper_bound: bool,
}

#[derive(Default)]
struct Fold {
    started: Option<Observation>,
    broke: Option<Observation>,
    previous: Option<CipherState>,
    idle_known: bool,
    last_end: Option<i32>,
}

impl Fold {
    fn reset(&mut self) {
        self.previous = None;
        self.idle_known = false;
    }

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
const FRESH: u8 = 8;

#[derive(Default)]
struct PairSteps {
    hours: Vec<i32>,
    amounts: Vec<i64>,
    flags: Vec<u8>,
}

impl PairSteps {
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

pub struct CipherHistory {
    last: i32,
    stretches: Vec<(i32, i32)>,
    pairs: BTreeMap<u64, PairSteps>,
    strengths: HashMap<u32, Vec<(i32, Option<i64>)>>,
}

impl CipherHistory {
    fn read(query: &Query<'_>) -> io::Result<Self> {
        let recording = query.replay.recording();
        let mut out = Self { last: query.timeline.last, stretches: Vec::new(), pairs: BTreeMap::new(), strengths: HashMap::new() };
        for stretch in &query.timeline.stretches {
            out.stretches.push((stretch.first, stretch.last));
            let fresh = if matches!(stretch.join, Join::Load { .. }) { 0 } else { FRESH };
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

    fn amount_at(&self, key: u64, hour: i32, now: i32) -> Result<i64, String> {
        let stretch = self.stretches.iter().find(|(first, last)| *first <= now && now <= *last);
        match stretch {
            Some(&(first, _)) if first <= hour => {}
            Some(_) => return Err("Not recorded 7 days earlier: the recording breaks within the last 7 days".into()),
            None => return Err("Not recorded 7 days earlier: this hour is not in the cipher history".into()),
        }
        let none = || String::from("Not recorded 7 days earlier: the pair has no reading then");
        let steps = self.pairs.get(&key).ok_or_else(none)?;
        let at = steps.hours.partition_point(|&step| step <= hour).checked_sub(1).ok_or_else(none)?;
        if steps.flags[at] & KNOWN == 0 {
            return Err("Not recorded 7 days earlier: the pair's reading then is not recorded".into());
        }
        Ok(steps.amounts[at])
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AttemptStep {
    hour: i32,
    started: Option<Observation>,
    broke: Option<Observation>,
}

pub struct Attempts(HashMap<u64, Vec<AttemptStep>>);

impl Attempts {
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

pub struct Ciphers {
    replay: Arc<Replay>,
    index: Arc<Index>,
    held: Arc<Mutex<Held>>,
}

impl Ciphers {
    pub fn of(source: &Source) -> Self {
        Self { replay: source.replay(), index: dashboard_base::queries::index(source), held: source.state::<Held>() }
    }

    fn history(&self, branch: u64) -> io::Result<Arc<CipherHistory>> {
        if let Some(found) = self.held.lock().unwrap_or_else(PoisonError::into_inner).histories.get(&branch) {
            return Ok(Arc::clone(found));
        }
        let made = Arc::new(CipherHistory::read(&self.index.query(&self.replay, branch)?)?);
        self.held.lock().unwrap_or_else(PoisonError::into_inner).histories.insert(branch, Arc::clone(&made));
        Ok(made)
    }

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
        CipherHistory { last, stretches: vec![(0, last)], pairs: pairs.into_iter().collect(), strengths: strengths.into_iter().collect() }
    }

    const fn seen(hour: i32, upper_bound: bool) -> Option<Observation> {
        Some(Observation { hour, upper_bound })
    }

    #[test]
    fn a_pairs_attempt_at_an_hour_is_the_fold_of_its_steps_with_its_targets_threshold_as_it_stood() {
        let key = pair_key(1, 2);
        let pair = steps(&[(0, 0, KNOWN | FRESH), (20, 0, KNOWN | WORKING), (30, 300_000, KNOWN | WORKING), (60, 900_000, KNOWN | WORKING)]);
        let strength = vec![(0, Some(300_000)), (40, Some(800_000))];
        let attempts = history(vec![(key, pair)], vec![(2, strength)], 99).attempts(Some(DEFINES));
        assert_eq!(attempts.at(key, 10), (None, None));
        assert_eq!(attempts.at(key, 25), (seen(20, true), None));
        assert_eq!(attempts.at(key, 35), (seen(20, true), seen(30, false)));
        assert_eq!(attempts.at(key, 99), (seen(20, true), seen(30, false)));
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
                week_change_tenths: None,
                week_change_unknown: None,
            },
        }
    }

    #[test]
    fn the_race_puts_broken_ciphers_first_then_progress_then_the_exact_share_and_unknown_progress_last() {
        let mut race = [
            ranked("GER", "ENG", 400, Some(1_000), Some(false)),
            ranked("ENG", "GER", 100, None, None),
            ranked("SOV", "GER", 900, Some(1_000), Some(false)),
            ranked("ITA", "FRA", 1_000, Some(1_000), Some(true)),
            ranked("JAP", "USA", 4_049, Some(10_000), Some(false)),
        ];
        race.sort_by(race_order);
        let owners: Vec<&str> = race.iter().map(|item| item.pair.owner.tag.as_deref().unwrap()).collect();
        assert_eq!(owners, ["ITA", "SOV", "JAP", "GER", "ENG"]);
    }

    #[test]
    fn the_week_change_is_witnessed_168_hours_earlier_in_the_same_stretch_or_named_unknown() {
        let key = pair_key(1, 2);
        let pair = steps(&[(0, 0, KNOWN | FRESH), (100, 0, KNOWN | WORKING), (150, 0, 0), (160, 2_000, KNOWN | WORKING), (400, 5_000, KNOWN | WORKING)]);
        let mut held = history(vec![(key, pair)], vec![], 600);
        held.stretches = vec![(0, 299), (300, 600)];
        assert_eq!(held.amount_at(key, 120, 288), Ok(0));
        assert_eq!(held.amount_at(key, 200, 368), Err("Not recorded 7 days earlier: the recording breaks within the last 7 days".into()));
        assert!(held.amount_at(key, 155, 290).is_err(), "an unknown reading then is not replaced by the nearest one held");
        assert!(held.amount_at(pair_key(9, 9), 120, 288).is_err());
        assert!(held.amount_at(key, 500, 700).is_err(), "an hour outside the history has no reading");
        assert_eq!(week_change(2_400, Ok(1_000), &Ok(10_000)), Ok(140));
        assert_eq!(week_change(1_000, Ok(1_049), &Ok(10_000)), Ok(-5));
        assert_eq!(week_change(1_000, Ok(1_055), &Ok(10_000)), Ok(-6));
        assert!(week_change(1_000, Ok(0), &Err("no strength".into())).is_err());
        assert!(week_change(1_000, Err("not then".into()), &Ok(10_000)).is_err());
    }

}
