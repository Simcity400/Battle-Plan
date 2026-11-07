//! Sparse hourly cipher races. Missing operands stay unknown; observations never
//! cross a gap. The game's unrecorded date is replaced by a dated observation.
use dashboard_base::{
    products::{MAX_PRODUCT_REPLY_BYTES, Product},
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_text::{
    catalogue::Refusals,
    cryptology,
    metadata::{self, recorded_rows},
};
use replay_data::{rows::CountryRow, rows::CryptologyRow, rows::TableId, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};
use ts_rs::TS;

#[cfg(any(test, feature = "test-support"))]
use replay::campaign::Replay;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::AtomicBool;

pub mod prepared;

#[cfg(any(test, feature = "test-support"))]
mod oracle;
#[cfg(any(test, feature = "test-support"))]
use oracle::scan;

pub struct Decryption;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecryptionParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CipherObservation {
    pub hour: i32,
    /// Already underway at the first read, after a gap, or after an unknown predicate.
    pub upper_bound: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DecryptionRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub start_amount_raw: String,
    pub step_raw_per_day: String,
    pub threshold_raw: Option<String>,
    pub threshold_unknown: Option<String>,
    /// Countdown at first_hour; reconstructed only at observed integer hours.
    pub bonus_days: i32,
    pub bonus_days_step_per_day: i32,
    /// Low byte: native decrypting; high byte: native bonus_active.
    pub state_flags: u16,
    pub broken: Option<bool>,
    /// Start observation of the attempt current during this run, never a prior episode.
    pub started: Option<CipherObservation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub preparing: Vec<String>,
}

impl DecryptionRun {
    fn amount_at(&self, hour: i32) -> Option<i64> {
        if !(self.first_hour..=self.last_hour).contains(&hour) {
            return None;
        }
        let days = (i64::from(hour) - i64::from(self.first_hour)) / 24;
        let amount = self.start_amount_raw.parse::<i64>().ok()?;
        let step = self.step_raw_per_day.parse::<i64>().ok()?;
        i64::try_from(i128::from(amount) + i128::from(step) * i128::from(days)).ok()
    }

    fn bonus_days_at(&self, hour: i32) -> Option<i32> {
        if !(self.first_hour..=self.last_hour).contains(&hour) {
            return None;
        }
        let days = (i64::from(hour) - i64::from(self.first_hour)) / 24;
        i32::try_from(i64::from(self.bonus_days) + i64::from(self.bonus_days_step_per_day) * days)
            .ok()
    }

    fn decrypting(&self) -> u8 {
        self.state_flags as u8
    }

    fn bonus_active(&self) -> u8 {
        (self.state_flags >> 8) as u8
    }

    fn work_at(&self, hour: i32) -> bool {
        self.decrypting() != 0 || self.amount_at(hour).is_some_and(|value| value > 0)
    }

    /// The daily update adds the fixed daily share (power/count, signed division),
    /// or decrements the active bonus countdown. Rates are learned from reads,
    /// never assumed: every hourly value must match. A run starts at its
    /// observed update phase; floor days retain plateaus. Whether `next` extends it, and
    /// then whether that changed its steps.
    fn extend(&mut self, next: &Self) -> Option<bool> {
        let hour = next.first_hour;
        if self.last_hour.checked_add(1) != Some(hour)
            || self.state_flags != next.state_flags
            || self.threshold_raw != next.threshold_raw
            || self.threshold_unknown != next.threshold_unknown
            || self.broken != next.broken
            || self.started != next.started
            || self.work_at(self.last_hour) != next.work_at(hour)
        {
            return None;
        }
        let start = self.amount_at(self.first_hour)?;
        let amount = next.amount_at(hour)?;
        let days = (i64::from(hour) - i64::from(self.first_hour)) / 24;
        let held = self
            .step_raw_per_day
            .parse::<i64>()
            .expect("stored raw step");
        let mut step = held;
        let mut bonus_step = self.bonus_days_step_per_day;
        // The first complete daily plateau can establish a step. A first read
        // partway through a day stays a separate prefix, aligning the new run.
        if i64::from(hour) - i64::from(self.first_hour) == 24 && step == 0 && bonus_step == 0 {
            step = amount.checked_sub(start)?;
            bonus_step = next.bonus_days.checked_sub(self.bonus_days)?;
        }
        if i128::from(start) + i128::from(step) * i128::from(days) != i128::from(amount)
            || i64::from(self.bonus_days) + i64::from(bonus_step) * days
                != i64::from(next.bonus_days_at(hour).expect("observed countdown"))
        {
            return None;
        }
        // The stored step is its value as written: only another value is written again.
        let changed = step != held || bonus_step != self.bonus_days_step_per_day;
        if step != held {
            self.step_raw_per_day = step.to_string();
        }
        self.bonus_days_step_per_day = bonus_step;
        self.last_hour = hour;
        Some(changed)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DecryptionTagRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// None when native tag bytes cannot be represented by the JSON/name contract.
    pub owner_tag: Option<String>,
    pub target_tag: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DecryptionPair {
    /// Canonical slots, resolved from each hour's CountryLinks, never tag concatenations.
    pub owner_id: String,
    pub target_id: String,
    pub runs: Vec<DecryptionRun>,
    pub tag_runs: Vec<DecryptionTagRun>,
    pub breaks: Vec<CipherObservation>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DecryptionCoverage {
    pub first_hour: i32,
    pub last_hour: i32,
    pub complete: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DecryptionReply {
    pub branch_id: String,
    pub pairs: Vec<DecryptionPair>,
    pub coverage: Vec<DecryptionCoverage>,
    pub warnings: Vec<String>,
}

impl Product for Decryption {
    const KEY: &'static str = "decryption";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> =
        Some(&super::intelligence_prepared::served::PLAN);
    type Params = DecryptionParams;
    type Reply = DecryptionReply;

    fn types() -> Vec<String> {
        vec![
            DecryptionParams::decl(),
            CipherObservation::decl(),
            DecryptionRun::decl(),
            DecryptionTagRun::decl(),
            DecryptionPair::decl(),
            DecryptionCoverage::decl(),
            DecryptionReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid branch")
    }

    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        metadata::check_cancel(cancelled)?;
        let mut warnings = Vec::new();
        let defines = defines(source, context, &mut warnings);
        let mut reply = scan(
            &source.replay(),
            params.branch_id.parse().expect("validated branch"),
            defines.as_ref().copied().map_err(String::as_str),
            cancelled,
        )?;
        reply.warnings.extend(warnings);
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub(super) fn defines(
    source: &Source,
    context: &LoadContext,
    warnings: &mut Vec<String>,
) -> Result<cryptology::Defines, String> {
    // A refused stack leaves thresholds unknown and captured amounts intact.
    let result = (|| {
        let options = context.content().map_err(|error| error.to_string())?;
        dashboard_base::content::check_replay(
            source.replay_ref(),
            options.allow_unverified,
            warnings,
        )
        .map_err(|error| error.to_string())?;
        let content = source
            .content(&options.options)
            .map_err(|error| error.to_string())?;
        content_defines(&content.layers)
    })();
    if let Err(reason) = &result {
        warnings.push(format!("Cipher thresholds unknown: {reason}"));
    }
    result
}

pub fn content_defines(
    layers: &installed_content_core::ResolvedLayers,
) -> Result<cryptology::Defines, String> {
    layers
        .derived::<dashboard_text::catalogue::CachedResult<cryptology::Defines>>()
        .get_or_init(|| {
            #[cfg(test)]
            super::intelligence_prepared::work(|work| work.content_loads += 1);
            cryptology::Defines::read(&installed_content_core::defines::read(layers)?)
                .map_err(dashboard_core::invalid)
        })
        .map(|defines| *defines)
        .map_err(|error| error.to_string())
}

fn table_reason(frame: &DecodedFrame, table: TableId) -> Option<String> {
    match &*frame.tables[table.index()] {
        TableView::Complete(_) => None,
        TableView::Truncated { missing, .. } => Some(format!(
            "{table:?}: {missing} rows not recorded (truncated capture)"
        )),
        TableView::Unavailable { reason } => Some(format!("{table:?}: {reason}")),
        TableView::NotRecorded { mode } => {
            Some(format!("{table:?}: not recorded in a {mode} recording"))
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Scan {
    #[serde(with = "dashboard_core::map_pairs")]
    pairs: BTreeMap<(u32, u32), PairScan>,
    coverage: Vec<DecryptionCoverage>,
    previous_complete: Option<i32>,
    refusals: Refusals,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    native_countries: super::operative_rows::CountryInputs,
    /// The row path's reads ([`Self::frame`]).
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    reader: Reader,
}

/// How a target's cipher threshold is known at a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(crate) enum Threshold {
    Known(i64),
    /// The cipher defines were refused.
    Defines,
    NoOwner,
    Unrecorded,
}

impl Threshold {
    fn result<'a>(self, defines: Result<cryptology::Defines, &'a str>) -> Result<i64, &'a str> {
        match self {
            Self::Known(value) => Ok(value),
            Self::Defines => Err(defines.err().unwrap_or_default()),
            Self::NoOwner => Err("RaidIntelInputs has no authenticated target cryptology owner"),
            Self::Unrecorded => Err("target RaidIntelInputs was not recorded at this hour"),
        }
    }
}

/// What the hour's countries join to a Cryptology row.
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(crate) struct Joined {
    key: (u32, u32),
    /// A tag the JSON contract cannot name keeps its bytes.
    owner_tag: Result<String, Vec<u8>>,
    target_tag: Result<String, Vec<u8>>,
    threshold: Threshold,
    /// The progress division overflows.
    overflow: bool,
}

/// A Cryptology row as read: unjoined when a country link or country is not recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(crate) struct PairRead {
    row: CryptologyRow,
    joined: Option<Joined>,
}

impl PairRead {
    pub(crate) fn key(&self) -> (u32, u32) {
        (self.row.owner_country, self.row.target_country)
    }

    /// The canonical slots it joins, where joined.
    fn slots(&self) -> Option<(u32, u32)> {
        self.joined.as_ref().map(|joined| joined.key)
    }

    pub(crate) fn heap(&self) -> usize {
        self.joined.as_ref().map_or(0, |joined| {
            [&joined.owner_tag, &joined.target_tag]
                .into_iter()
                .map(|tag| match tag {
                    Ok(tag) => tag.capacity(),
                    Err(bytes) => bytes.capacity(),
                })
                .sum()
        })
    }
}

/// What the cipher scan reads of an hour.
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(crate) enum CipherRead {
    /// No hour was read.
    Unread,
    /// The countries could not be identified.
    Countries(String),
    /// A table the scan reads was not recorded.
    Failed(io::ErrorKind, String),
    Rows {
        /// Why the cipher table is incomplete.
        incomplete: Option<String>,
        pairs: Vec<PairRead>,
    },
}

impl Default for CipherRead {
    fn default() -> Self {
        Self::Unread
    }
}

/// Why the coverage of an hour holding an unjoined cipher entry is incomplete.
const UNJOINED: &str = "Cryptology country link or country is not recorded";

/// The cipher scan's read as the ordered emit holds it: each pair by its raw countries, and
/// each joined pair also by its canonical slots.
#[derive(Default)]
pub(crate) struct CipherHeld {
    /// The read; a read of rows keeps its pairs in `pairs` instead.
    pub(crate) read: CipherRead,
    pairs: BTreeMap<(u32, u32), PairRead>,
    slots: HashMap<(u32, u32), BTreeSet<(u32, u32)>>,
    /// The slots more than one pair joins.
    crowded: BTreeSet<(u32, u32)>,
    unjoined: usize,
}

/// What a read changed of the pairs held.
#[derive(Default)]
pub(crate) struct CipherTouched {
    /// The read is another kind, or was taken whole: every pair is gone through.
    pub(crate) all: bool,
    /// The raw keys of the pairs that changed or came.
    rows: Vec<(u32, u32)>,
    /// The slots a changed or gone pair joined before.
    slots: Vec<(u32, u32)>,
}

impl CipherHeld {
    /// Takes `read` in place of the read held, telling `touched` what it changed.
    pub(crate) fn replace(&mut self, read: CipherRead, touched: &mut CipherTouched) {
        match read {
            CipherRead::Rows { incomplete, pairs }
                if matches!(self.read, CipherRead::Rows { .. }) =>
            {
                let mut gone: BTreeSet<(u32, u32)> = self.pairs.keys().copied().collect();
                for pair in pairs {
                    gone.remove(&pair.key());
                    if self.pairs.get(&pair.key()) != Some(&pair) {
                        self.set(pair, touched);
                    }
                }
                for key in gone {
                    self.remove(key, touched);
                }
                self.read = CipherRead::Rows {
                    incomplete,
                    pairs: Vec::new(),
                };
            }
            CipherRead::Rows { incomplete, pairs } => {
                self.clear();
                for pair in pairs {
                    self.set(pair, touched);
                }
                touched.all = true;
                self.read = CipherRead::Rows {
                    incomplete,
                    pairs: Vec::new(),
                };
            }
            read => {
                self.clear();
                touched.all = true;
                self.read = read;
            }
        }
    }

    /// Why the held rows' table is incomplete, where the read is of rows.
    pub(crate) fn set_incomplete(&mut self, reason: Option<String>) {
        if let CipherRead::Rows { incomplete, .. } = &mut self.read {
            *incomplete = reason;
        }
    }

    fn clear(&mut self) {
        self.pairs.clear();
        self.slots.clear();
        self.crowded.clear();
        self.unjoined = 0;
    }

    /// Holds `pair` in place of the pair of its raw countries.
    pub(crate) fn set(&mut self, pair: PairRead, touched: &mut CipherTouched) {
        let key = pair.key();
        let slots = pair.slots();
        if slots.is_none() {
            self.unjoined += 1;
        }
        if let Some(old) = self.pairs.insert(key, pair) {
            self.unlink(key, &old, touched);
        }
        if let Some(slot) = slots {
            let keys = self.slots.entry(slot).or_default();
            keys.insert(key);
            if keys.len() > 1 {
                self.crowded.insert(slot);
            }
        }
        touched.rows.push(key);
    }

    pub(crate) fn remove(&mut self, key: (u32, u32), touched: &mut CipherTouched) {
        if let Some(old) = self.pairs.remove(&key) {
            self.unlink(key, &old, touched);
        }
    }

    fn unlink(&mut self, key: (u32, u32), old: &PairRead, touched: &mut CipherTouched) {
        let Some(slot) = old.slots() else {
            self.unjoined -= 1;
            return;
        };
        touched.slots.push(slot);
        if let Some(keys) = self.slots.get_mut(&slot) {
            keys.remove(&key);
            match keys.len() {
                0 => {
                    self.slots.remove(&slot);
                }
                1 => {
                    self.crowded.remove(&slot);
                }
                _ => {}
            }
        }
    }
}

/// Reads the cipher scan's tables, keeping each target's cryptology strength until its inputs
/// change.
#[derive(Default)]
pub(crate) struct Reader {
    raw_strength: Option<BTreeMap<u32, Option<i64>>>,
    strength: BTreeMap<u32, Option<i64>>,
}

impl Reader {
    /// What `frame` gives the scan, where `inputs` are its countries; `changes` None reads
    /// every table again.
    pub(crate) fn read(
        &mut self,
        frame: &DecodedFrame,
        defines: Option<cryptology::Defines>,
        inputs: &super::operative_rows::CountryInputs,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> CipherRead {
        match self.rows(frame, defines, inputs, changes) {
            Ok(read) => read,
            Err(error) => CipherRead::Failed(error.kind(), error.to_string()),
        }
    }

    fn rows(
        &mut self,
        frame: &DecodedFrame,
        defines: Option<cryptology::Defines>,
        inputs: &super::operative_rows::CountryInputs,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<CipherRead> {
        let countries = match inputs.strict.as_ref().expect("country operands reached") {
            Ok(countries) => countries,
            Err(error) => return Ok(CipherRead::Countries(error.clone())),
        };
        let links = inputs.links.as_deref().expect("complete cipher links");
        let incomplete = table_reason(frame, TableId::Cryptology);
        let raw_changed = self.raw_strength.is_none()
            || changes.is_none_or(|changes| changes.rows_changed[TableId::RaidIntelInputs.index()]);
        if raw_changed {
            let intel = recorded_rows::<tables::RaidIntelInputs>(frame)?;
            #[cfg(test)]
            super::intelligence_prepared::work(|work| work.row_visits += intel.len());
            self.raw_strength = Some(
                intel
                    .iter()
                    .map(|row| (row.country, row.crypto_strength))
                    .collect(),
            );
        }
        if inputs.changed || raw_changed {
            self.strength = self
                .raw_strength
                .as_ref()
                .expect("target cryptology operands")
                .iter()
                .filter_map(|(&raw, &value)| {
                    metadata::link_slot(links, raw).map(|slot| (slot, value))
                })
                .collect();
        }
        let rows = recorded_rows::<tables::Cryptology>(frame)?;
        let mut pairs = Vec::with_capacity(rows.len());
        for row in rows {
            #[cfg(test)]
            super::intelligence_prepared::work(|work| work.row_visits += 1);
            let identity = metadata::link_slot(links, row.owner_country)
                .zip(metadata::link_slot(links, row.target_country));
            let named = identity.and_then(|(owner, target)| {
                countries
                    .get(&owner)
                    .zip(countries.get(&target))
                    .map(|(a, b)| ((owner, target), (a, b)))
            });
            let joined = named.map(|(key, (owner, target))| {
                let tag = |country: &CountryRow| match std::str::from_utf8(&country.tag) {
                    Ok(tag) if !tag.is_empty() && !tag.contains('\0') => Ok(tag.to_owned()),
                    _ => Err(country.tag.clone()),
                };
                let threshold = match (defines, self.strength.get(&key.1)) {
                    (None, _) => Threshold::Defines,
                    (Some(defines), Some(Some(value))) => {
                        Threshold::Known(cryptology::threshold(*value, defines))
                    }
                    (Some(_), Some(None)) => Threshold::NoOwner,
                    (Some(_), None) => Threshold::Unrecorded,
                };
                let overflow = match threshold {
                    Threshold::Known(required) => cryptology::progress_percent(
                        row.amount_raw,
                        row.bonus_active,
                        required,
                    )
                    .is_none(),
                    _ => false,
                };
                Joined {
                    key,
                    owner_tag: tag(owner),
                    target_tag: tag(target),
                    threshold,
                    overflow,
                }
            });
            pairs.push(PairRead {
                row: row.clone(),
                joined,
            });
        }
        Ok(CipherRead::Rows { incomplete, pairs })
    }
}

#[derive(Serialize, Deserialize)]
struct PairScan {
    pair: DecryptionPair,
    idle_known: bool,
}

struct ObservedPair {
    key: (u32, u32),
    starts: bool,
    changed: bool,
}

impl Scan {
    fn cover(&mut self, hour: i32, reason: Option<String>) {
        let complete = reason.is_none();
        if let Some(last) = self.coverage.last_mut()
            && last.last_hour.checked_add(1) == Some(hour)
            && last.complete == complete
            && last.reason == reason
        {
            last.last_hour = hour;
        } else {
            self.coverage.push(DecryptionCoverage {
                first_hour: hour,
                last_hour: hour,
                complete,
                reason,
            });
        }
        self.previous_complete = complete.then_some(hour);
    }

    /// Observes the scan's `read` of `hour`, every pair of it, `defines` naming why thresholds
    /// are unknown.
    #[cfg(any(test, feature = "test-support"))]
    fn observe_read(
        &mut self,
        hour: i32,
        read: &CipherRead,
        defines: Result<cryptology::Defines, &str>,
    ) -> io::Result<Vec<ObservedPair>> {
        let previous_complete =
            self.previous_complete.and_then(|last| last.checked_add(1)) == Some(hour);
        let (incomplete, pairs) = match read {
            CipherRead::Unread => return Err(io::Error::other("The cipher tables were not read")),
            CipherRead::Countries(error) => {
                self.cover(
                    hour,
                    Some(format!("Country identity not recorded: {error}")),
                );
                return Ok(Vec::new());
            }
            CipherRead::Failed(kind, message) => return Err(io::Error::new(*kind, message.clone())),
            CipherRead::Rows { incomplete, pairs } => (incomplete, pairs),
        };
        let mut incomplete = incomplete.clone();
        let mut observed = Vec::with_capacity(pairs.len());
        for pair in pairs {
            match self.observe_row(hour, pair, defines, previous_complete) {
                Some(pair) => observed.push(pair),
                None => incomplete = Some(UNJOINED.into()),
            }
        }
        self.cover(hour, incomplete);
        Ok(observed)
    }

    /// Observes `pair` as read at `hour`, noting what it refuses: none where its countries are
    /// not joined.
    fn observe_row(
        &mut self,
        hour: i32,
        PairRead { row, joined }: &PairRead,
        defines: Result<cryptology::Defines, &str>,
        previous_complete: bool,
    ) -> Option<ObservedPair> {
        let Some(joined) = joined else {
            self.refusals.note(
                UNJOINED,
                format!("{} → {}", row.owner_country, row.target_country),
            );
            return None;
        };
        let key = joined.key;
        let mut tag = |tag: &Result<String, Vec<u8>>, slot: u32| match tag {
            Ok(tag) => Some(tag.clone()),
            Err(bytes) => {
                self.refusals.note(
                        "country tag cannot be represented as a nonempty non-NUL JSON string; cipher data retained with country ID",
                        format!("country {slot}, tag bytes {bytes:?}"),
                    );
                None
            }
        };
        let owner_tag = tag(&joined.owner_tag, key.0);
        let target_tag = tag(&joined.target_tag, key.1);
        let threshold = joined.threshold.result(defines);
        if let Err(reason) = threshold {
            self.refusals.note(
                format!("progress unknown: {reason}"),
                format!("country {}", key.1),
            );
        }
        let run = run(hour, row, threshold);
        if joined.overflow {
            self.refusals.note(
                "progress unknown: native signed division overflow",
                format!("country {} → country {}", key.0, key.1),
            );
        }
        let (starts, changed) = Self::observe(
            &mut self.pairs,
            key,
            run,
            DecryptionTagRun {
                first_hour: hour,
                last_hour: hour,
                owner_tag,
                target_tag,
            },
            previous_complete,
        );
        Some(ObservedPair {
            key,
            starts,
            changed,
        })
    }

    fn observe(
        pairs: &mut BTreeMap<(u32, u32), PairScan>,
        key: (u32, u32),
        mut run: DecryptionRun,
        tags: DecryptionTagRun,
        previous_complete: bool,
    ) -> (bool, bool) {
        let state = pairs.entry(key).or_insert_with(|| PairScan {
            pair: DecryptionPair {
                owner_id: key.0.to_string(),
                target_id: key.1.to_string(),
                runs: Vec::new(),
                tag_runs: Vec::new(),
                breaks: Vec::new(),
            },
            idle_known: false,
        });
        let pair = &mut state.pair;
        let previous = pair
            .runs
            .last()
            .filter(|last| last.last_hour.checked_add(1) == Some(run.first_hour));
        run.started = attempt_start(previous, &run, previous_complete, &mut state.idle_known);
        // Activation scales amount; expiry clears active. Every false -> true
        // transition is a new break; a gap/unknown bounds it.
        let break_changed =
            run.broken == Some(true) && previous.is_none_or(|last| last.broken != Some(true));
        if break_changed {
            let known_before = match previous {
                Some(last) => last.broken == Some(false),
                None => previous_complete,
            };
            pair.breaks.push(CipherObservation {
                hour: run.first_hour,
                upper_bound: !known_before,
            });
        }
        let tags_changed = if let Some(last) = pair.tag_runs.last_mut()
            && last.last_hour.checked_add(1) == Some(tags.first_hour)
            && last.owner_tag == tags.owner_tag
            && last.target_tag == tags.target_tag
        {
            last.last_hour = tags.last_hour;
            false
        } else {
            pair.tag_runs.push(tags);
            true
        };
        if let Some(last) = pair.runs.last_mut()
            && let Some(steps_changed) = last.extend(&run)
        {
            return (false, tags_changed || break_changed || steps_changed);
        }
        pair.runs.push(run);
        (true, true)
    }
}

fn attempt_start(
    previous: Option<&DecryptionRun>,
    run: &DecryptionRun,
    previous_complete: bool,
    idle_known: &mut bool,
) -> Option<CipherObservation> {
    let amount = run.amount_at(run.first_hour).expect("observed amount");
    let Some(previous) = previous else {
        // A gap cannot prove continuity with an earlier attempt.
        *idle_known = previous_complete;
        return (run.work_at(run.first_hour) || run.bonus_active() != 0).then_some(
            CipherObservation {
                hour: run.first_hour,
                upper_bound: !previous_complete,
            },
        );
    };
    let previous_amount = previous
        .amount_at(previous.last_hour)
        .expect("observed amount");
    // Starting sets decrypting; stopping retains amount, including a zero-progress
    // pause. Activation stops work and scales amount; expiry clears active but
    // retains that residual. Clearing stopped work by script also ends an attempt.
    let ended = (previous.bonus_active() != 0 && run.bonus_active() == 0)
        || (previous_amount > 0 && amount <= 0 && run.decrypting() == 0 && run.bonus_active() == 0);
    if ended {
        *idle_known = true;
    }
    if !ended && previous.started.is_some() {
        return previous.started.clone();
    }
    // An initial stopped zero may be paused; only known absence or an observed
    // end proves idle. Scripted work can start without the flag.
    // An unchanged residual after expiry is not a new start observation.
    (run.decrypting() != 0 || run.bonus_active() != 0 || (amount > 0 && amount > previous_amount))
        .then_some(CipherObservation {
            hour: run.first_hour,
            upper_bound: !*idle_known,
        })
}

fn run(hour: i32, row: &CryptologyRow, threshold: Result<i64, &str>) -> DecryptionRun {
    let required = threshold.ok();
    DecryptionRun {
        first_hour: hour,
        last_hour: hour,
        start_amount_raw: row.amount_raw.to_string(),
        step_raw_per_day: "0".into(),
        threshold_raw: required.map(|v| v.to_string()),
        threshold_unknown: threshold.err().map(str::to_owned),
        bonus_days: row.bonus_days,
        bonus_days_step_per_day: 0,
        state_flags: u16::from(row.decrypting) | (u16::from(row.bonus_active) << 8),
        broken: cryptology::fully_decrypted(row.amount_raw, row.bonus_active, required),
        started: None,
        preparing: Vec::new(),
    }
}

fn check_reply_budget(reply: &DecryptionReply) -> io::Result<()> {
    dashboard_core::transport::json_len(reply, MAX_PRODUCT_REPLY_BYTES)
        .map(|_| ())
        .ok_or_else(|| {
            io::Error::other("Decryption history exceeds the complete-response byte budget")
        })
}

#[cfg(test)]
mod tests;
