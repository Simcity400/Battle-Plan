//! Sparse hourly cipher races. Missing operands stay unknown; observations never
//! cross a gap. The game's unrecorded date is replaced by a dated observation.
use super::{LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, recorded_rows};
use crate::{Source, catalogue::Refusals, cryptology, metadata};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, TableView},
    rows::{CountryRow, CryptologyRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};
use ts_rs::TS;

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
    /// observed update phase; floor days retain plateaus.
    fn extend(&mut self, next: &Self) -> bool {
        let hour = next.first_hour;
        if self.last_hour.checked_add(1) != Some(hour)
            || self.state_flags != next.state_flags
            || self.threshold_raw != next.threshold_raw
            || self.threshold_unknown != next.threshold_unknown
            || self.broken != next.broken
            || self.started != next.started
            || self.work_at(self.last_hour) != next.work_at(hour)
        {
            return false;
        }
        let Some(start) = self.amount_at(self.first_hour) else {
            return false;
        };
        let Some(amount) = next.amount_at(hour) else {
            return false;
        };
        let days = (i64::from(hour) - i64::from(self.first_hour)) / 24;
        let mut step = self
            .step_raw_per_day
            .parse::<i64>()
            .expect("stored raw step");
        let mut bonus_step = self.bonus_days_step_per_day;
        // The first complete daily plateau can establish a step. A first read
        // partway through a day stays a separate prefix, aligning the new run.
        if i64::from(hour) - i64::from(self.first_hour) == 24 && step == 0 && bonus_step == 0 {
            let Some(delta) = amount.checked_sub(start) else {
                return false;
            };
            let Some(delta_days) = next.bonus_days.checked_sub(self.bonus_days) else {
                return false;
            };
            step = delta;
            bonus_step = delta_days;
        }
        if i128::from(start) + i128::from(step) * i128::from(days) != i128::from(amount)
            || i64::from(self.bonus_days) + i64::from(bonus_step) * days
                != i64::from(next.bonus_days_at(hour).expect("observed countdown"))
        {
            return false;
        }
        self.step_raw_per_day = step.to_string();
        self.bonus_days_step_per_day = bonus_step;
        self.last_hour = hour;
        true
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

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        metadata::check_cancel(cancelled)?;
        let mut warnings = Vec::new();
        // Content verification is the same as other products. A refusal leaves
        // progress unknown while preserving the captured amounts and states.
        let defines = (|| {
            let options = context.content().map_err(|e| e.to_string())?;
            crate::content::check_replay(&source.replay(), options.allow_unverified, &mut warnings)
                .map_err(|e| e.to_string())?;
            let content = source
                .content(&options.options)
                .map_err(|e| e.to_string())?;
            content_defines(&content.layers)
        })();
        if let Err(reason) = &defines {
            warnings.push(format!("Cipher thresholds unknown: {reason}"));
        }
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

pub(crate) fn content_defines(
    layers: &installed_content::ResolvedLayers,
) -> Result<cryptology::Defines, String> {
    cryptology::Defines::read(&installed_content::defines::read(layers).map_err(|e| e.to_string())?)
}

fn table_reason(frame: &DecodedFrame, table: TableId) -> Option<String> {
    match &*frame.tables[table.index()] {
        TableView::Complete(_) => None,
        TableView::Truncated { missing, .. } => Some(format!(
            "{table:?}: {missing} rows not recorded (truncated capture)"
        )),
        TableView::Unavailable { reason } => Some(format!("{table:?}: {reason}")),
    }
}

#[derive(Default)]
struct Scan {
    pairs: BTreeMap<(u32, u32), PairScan>,
    coverage: Vec<DecryptionCoverage>,
    previous_complete: Option<i32>,
    refusals: Refusals,
}

struct PairScan {
    pair: DecryptionPair,
    idle_known: bool,
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

    fn frame(
        &mut self,
        frame: &DecodedFrame,
        defines: Result<cryptology::Defines, &str>,
    ) -> io::Result<()> {
        let hour = frame.hour;
        let previous_complete =
            self.previous_complete.and_then(|last| last.checked_add(1)) == Some(hour);
        let identities = (|| {
            let (countries, links) = metadata::country_tables(frame)?;
            let mut named = BTreeMap::<u32, &CountryRow>::new();
            for country in countries {
                named.insert(metadata::mapped_slot(country, links)?, country);
            }
            Ok::<_, io::Error>((named, links))
        })();
        let (countries, links) = match identities {
            Ok(found) => found,
            Err(error) => {
                self.cover(
                    hour,
                    Some(format!("Country identity not recorded: {error}")),
                );
                return Ok(());
            }
        };
        let mut incomplete = table_reason(frame, TableId::Cryptology);
        let intel = recorded_rows::<tables::RaidIntelInputs>(frame)?;
        let strength: BTreeMap<_, _> = intel
            .iter()
            .filter_map(|row| {
                metadata::link_slot(links, row.country).map(|slot| (slot, row.crypto_strength))
            })
            .collect();
        for row in recorded_rows::<tables::Cryptology>(frame)? {
            let identity = metadata::link_slot(links, row.owner_country)
                .zip(metadata::link_slot(links, row.target_country));
            let named = identity.and_then(|(owner, target)| {
                countries
                    .get(&owner)
                    .zip(countries.get(&target))
                    .map(|(a, b)| ((owner, target), (*a, *b)))
            });
            let Some((key, (owner, target))) = named else {
                let reason = "Cryptology country link or country is not recorded";
                self.refusals.note(
                    reason,
                    format!("{} → {}", row.owner_country, row.target_country),
                );
                incomplete = Some(reason.into());
                continue;
            };
            let mut tag = |country: &CountryRow, slot: u32| match std::str::from_utf8(&country.tag)
            {
                Ok(tag) if !tag.is_empty() && !tag.contains('\0') => Some(tag.to_owned()),
                _ => {
                    self.refusals.note(
                            "country tag cannot be represented as a nonempty non-NUL JSON string; cipher data retained with country ID",
                            format!("country {slot}, tag bytes {:?}", country.tag),
                        );
                    None
                }
            };
            let owner_tag = tag(owner, key.0);
            let target_tag = tag(target, key.1);
            let threshold = defines.and_then(|defines| match strength.get(&key.1) {
                Some(Some(value)) => Ok(cryptology::threshold(*value, defines)),
                Some(None) => Err("RaidIntelInputs has no authenticated target cryptology owner"),
                None => Err("target RaidIntelInputs was not recorded at this hour"),
            });
            if let Err(reason) = threshold {
                self.refusals.note(
                    format!("progress unknown: {reason}"),
                    format!("country {}", key.1),
                );
            }
            let run = run(hour, row, threshold);
            if threshold.is_ok_and(|required| {
                cryptology::progress_percent(row.amount_raw, run.bonus_active(), required).is_none()
            }) {
                self.refusals.note(
                    "progress unknown: native signed division overflow",
                    format!("country {} → country {}", key.0, key.1),
                );
            }
            self.observe(
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
        }
        self.cover(hour, incomplete);
        Ok(())
    }

    fn observe(
        &mut self,
        key: (u32, u32),
        mut run: DecryptionRun,
        tags: DecryptionTagRun,
        previous_complete: bool,
    ) {
        let state = self.pairs.entry(key).or_insert_with(|| PairScan {
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
        if run.broken == Some(true) && previous.is_none_or(|last| last.broken != Some(true)) {
            let known_before = match previous {
                Some(last) => last.broken == Some(false),
                None => previous_complete,
            };
            pair.breaks.push(CipherObservation {
                hour: run.first_hour,
                upper_bound: !known_before,
            });
        }
        if let Some(last) = pair.tag_runs.last_mut()
            && last.last_hour.checked_add(1) == Some(tags.first_hour)
            && last.owner_tag == tags.owner_tag
            && last.target_tag == tags.target_tag
        {
            last.last_hour = tags.last_hour;
        } else {
            pair.tag_runs.push(tags);
        }
        if let Some(last) = pair.runs.last_mut()
            && last.extend(&run)
        {
            return;
        }
        pair.runs.push(run);
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
    }
}

pub fn scan(
    replay: &Replay,
    branch: u64,
    defines: Result<cryptology::Defines, &str>,
    cancelled: &AtomicBool,
) -> io::Result<DecryptionReply> {
    if !replay.branches.iter().any(|item| item.id == branch) {
        return Err(crate::invalid("Unknown branch"));
    }
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Cryptology,
        TableId::RaidIntelInputs,
    ];
    let mut scan = Scan::default();
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        metadata::check_cancel(cancelled)?;
        let mut decoder = FrameDecoder::for_tables(&selected);
        let frames = match replay.read_block_selected(block, &selected) {
            Ok(frames) => frames,
            Err(error) => {
                scan.refusals.note(
                    format!("unreadable decryption block: {error}"),
                    block.first_hour.to_string(),
                );
                continue;
            }
        };
        for frame in frames {
            metadata::check_cancel(cancelled)?;
            let decoded = match decoder.advance(&frame).and_then(|decoded| {
                metadata::validate_campaign(decoded, replay)?;
                Ok(decoded)
            }) {
                Ok(decoded) => decoded,
                Err(error) => {
                    scan.refusals.note(
                        format!("unreadable decryption frame: {error}"),
                        frame.hour.to_string(),
                    );
                    break;
                }
            };
            scan.frame(decoded, defines)?;
        }
    }
    let reply = DecryptionReply {
        branch_id: branch.to_string(),
        pairs: scan.pairs.into_values().map(|state| state.pair).collect(),
        coverage: scan.coverage,
        warnings: scan.refusals.diagnostic_lines("cipher entries affected"),
    };
    check_reply_budget(&reply)?;
    Ok(reply)
}

fn check_reply_budget(reply: &DecryptionReply) -> io::Result<()> {
    crate::transport::json_len(reply, MAX_PRODUCT_REPLY_BYTES)
        .map(|_| ())
        .ok_or_else(|| {
            io::Error::other("Decryption history exceeds the complete-response byte budget")
        })
}

#[cfg(test)]
mod tests;
