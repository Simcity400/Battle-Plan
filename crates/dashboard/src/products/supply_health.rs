//! Division supply-icon states, counted at their hourly reads.
//!
//! Low supply is false for naval transfer (the pointer at +2F8), otherwise
//! effective supply < NMilitary.LOW_SUPPLY. Stored signed qwords hold the
//! received fixed5 fraction (+40), maximum (+600) and remaining (+610) grace.
//! Maximum grace starts with SUPPLY_GRACE (integer hours) * 100000; the hourly
//! division update passes remaining grace (+610) and its companion (+608) to
//! the grace updater. We read stored values, never recreate
//! their update or make them fresher. DivisionStatus records these exact
//! signed i64 operands (supply_ratio, supply_grace_maximum/remaining) and
//! DivisionLandInputs the naval-transfer presence, both from the same hour's
//! authenticated DivisionSeed (engine::carmy), keyed by full u64 handle,
//! hourly in cadence.rs.
//!
//! The icon substitutes fixed5 one when supply is not low; otherwise it uses the
//! received fraction. Strict < VERY_LOW selects frame 3, then strict < LOW selects
//! frame 2. Otherwise the supply gate selects frame 1 or hides the icon.
//! The tooltip uses the same gate for ARMY_UNIT_SUPPLY_DESC_OK/BAD and displays
//! the effective fraction. There is no separate zero-supply enum. The chart calls
//! very-low frame 3 "No supply", frames 1/2 "Strained", hidden "In supply": grace
//! can cover low received supply; frame 3 need not mean zero. Even unusual mod
//! thresholds retain the icon's comparison order.
//!
//! LOW_SUPPLY is fixed5; SUPPLY_GRACE is integer hours.
//! NGraphics.SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD and
//! SUPPLY_UNIT_COUNTER_LOW_THRESHOLD are f32 inputs. Their staged fixed5
//! conversion is shared by installed_content::defines::fixed5. Every value resolves
//! through the recorded content stack; province supply is no proxy.
//!
//! Owner: positive signed expeditionary_owner, else roster_owner, through
//! CountryLinks, matching DivisionCensus and Army size. Expeditionary transfer
//! preserves the sender before changing the division's host roster.
//! No player/AI filtering. Incomplete Divisions or
//! country links, unresolved owners, or any owned division without this hour's
//! status supply operands or land inputs breaks the country's whole stack.
//! Zero divisions is an exact zero denominator, not a 100% share.
//! metadata::slot_read dates every operand; equal consecutive reads compress
//! into runs without crossing gaps.
//!
//! Cost fixture: 5,000 countries, 50,000 divisions, 78,840 hourly reads. Each
//! division has a staggered 48-hour disruption every 30 days, with hourly grace
//! drain/refill (5.417% grace, 0.278% received-fraction changes/hour), 5% location
//! and 1% country-balance churn. Ownership stays unchanged; joins compare only
//! ownership facts. Decoded builder timings exclude fixture mutations/encoding.
//! Wire timings average 21 real 24-hour block scans, with 5% grace-row churn
//! and one manpower contributor per division, including selected-table decode.
//! Selection timings include complete reply byte counting; cached nine-year
//! histories scan no hours. New tags scan the branch once, sharing a per-branch
//! in-flight slot outside the cache mutex. Only current-branch country histories
//! remain cached, without a country/entry budget. The complete selected reply
//! refuses only past MAX_PRODUCT_REPLY_BYTES.

use super::{LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, recorded_rows, validate_tag};
use crate::{
    Source, content,
    metadata::{IdentityTrack, check_cancel, slot_read, tagged_countries, validate_campaign},
    transport::json_len,
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, ViewIdentity},
    rows::{DivisionLandInputsRow, DivisionRow, DivisionStatusRow, Recorded, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex, Weak, atomic::AtomicBool},
    time::Duration,
};
use ts_rs::TS;

pub struct SupplyHealth;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SupplyHealthParams {
    pub branch_id: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct SupplyHealthRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    /// Exact counts; divide by their sum only for geometry/formatting.
    pub no_supply: u32,
    pub strained: u32,
    pub in_supply: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SupplyHealthSeries {
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<super::country_history::CountryIdentityRun>>,
    pub runs: Vec<SupplyHealthRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SupplyHealthReply {
    pub branch_id: String,
    pub available: bool,
    pub reason: Option<String>,
    pub missing_inputs: Vec<String>,
    pub series: Vec<SupplyHealthSeries>,
    pub warnings: Vec<String>,
}

/// One division's supply-icon operands for an hour: DivisionStatus's received
/// fraction and grace, and DivisionLandInputs's naval-transfer presence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Supply {
    received: i64,
    maximum_grace: i64,
    remaining_grace: i64,
    naval_transfer: bool,
}

impl Supply {
    /// None when either table lacks the division or the grace was not
    /// recorded; such a division cannot be classified.
    fn of(status: &DivisionStatusRow, land: &DivisionLandInputsRow) -> Option<Self> {
        let (Recorded::Value(maximum_grace), Recorded::Value(remaining_grace)) =
            (status.supply_grace_maximum, status.supply_grace_remaining)
        else {
            return None;
        };
        Some(Self {
            received: status.supply_ratio,
            maximum_grace,
            remaining_grace,
            naval_transfer: land.naval_transfer,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Defines {
    grace: i32,
    low: i64,
    very_low_icon: i64,
    low_icon: i64,
}

pub(crate) fn check_content(layers: &installed_content::ResolvedLayers) -> io::Result<()> {
    content_defines(layers).map(|_| ())
}

fn content_defines(layers: &installed_content::ResolvedLayers) -> io::Result<Defines> {
    Defines::read(&installed_content::defines::read(layers)?)
}

impl Defines {
    fn read(defines: &installed_content::defines::Defines) -> io::Result<Self> {
        let grace = defines
            .number("NMilitary", "SUPPLY_GRACE")
            .and_then(|_| defines.int32("NMilitary", "SUPPLY_GRACE"));
        let low = defines.fixed5("NMilitary", "LOW_SUPPLY");
        let very_low_icon = defines.fixed5("NGraphics", "SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD");
        let low_icon = defines.fixed5("NGraphics", "SUPPLY_UNIT_COUNTER_LOW_THRESHOLD");
        let errors: Vec<String> = [
            grace.as_ref().err(),
            low.as_ref().err(),
            very_low_icon.as_ref().err(),
            low_icon.as_ref().err(),
        ]
        .into_iter()
        .flatten()
        .map(ToString::to_string)
        .collect();
        if !errors.is_empty() {
            return Err(crate::invalid(format!(
                "Supply health refused {} define(s): {}",
                errors.len(),
                errors.join("; ")
            )));
        }
        Ok(Self {
            grace: grace?,
            low: low?,
            very_low_icon: very_low_icon?,
            low_icon: low_icon?,
        })
    }

    fn effective(self, row: &Supply) -> i64 {
        if row.maximum_grace <= 0 {
            return 0;
        }
        let divisor = i64::from(self.grace) * 100_000;
        let exceptional = (row.remaining_grace as u64).wrapping_add(0x7fff_ffff_ffff_fffe)
            > 0xffff_ffff_ffff_fffc;
        let grace = if divisor == 0 || exceptional {
            0xffff_ffff
        } else {
            row.remaining_grace.wrapping_mul(100_000) / divisor
        };
        row.received.max(grace).clamp(0, 100_000)
    }

    fn has_low(self, row: &Supply) -> bool {
        !row.naval_transfer && self.effective(row) < self.low
    }

    /// Native icon frame; zero means hidden. Keep all four native arms.
    fn frame(self, row: &Supply) -> usize {
        let low = self.has_low(row);
        let received = if low { row.received } else { 100_000 };
        if received < self.very_low_icon {
            3
        } else if received < self.low_icon {
            2
        } else {
            usize::from(low)
        }
    }
}

fn owner(row: &DivisionRow) -> u32 {
    if (row.expeditionary_owner as i32) > 0 {
        row.expeditionary_owner
    } else {
        row.roster_owner
    }
}

#[derive(Default)]
struct Seen {
    identity: IdentityTrack,
    open: bool,
}

struct Scan {
    /// The recording's format, which dates its daily tables
    /// ([`crate::metadata::slot_read`]).
    format: replay::format::FormatVersion,
    series: Vec<SupplyHealthSeries>,
    selected: HashMap<String, usize>,
    seen: Vec<Seen>,
    identities: Vec<Option<(u32, u32)>>,
    owners: HashMap<u32, Option<usize>>,
    countries_from: Option<[ViewIdentity; 2]>,
    country_keys: Vec<(u32, u32, Vec<u8>)>,
    counts_from: Option<[ViewIdentity; 3]>,
    roster_from: Option<ViewIdentity>,
    roster_keys: Vec<(u64, u32)>,
    members: Vec<(u64, usize)>,
    owners_known: bool,
    counts: Vec<Option<[u32; 3]>>,
    available: bool,
}

impl Scan {
    fn new(tags: &[String]) -> Self {
        let mut selected = HashMap::new();
        let mut series = Vec::new();
        for tag in tags {
            if selected.contains_key(tag) {
                continue;
            }
            selected.insert(tag.clone(), series.len());
            series.push(SupplyHealthSeries {
                tag: tag.clone(),
                identities: None,
                runs: Vec::new(),
            });
        }
        let len = series.len();
        Self {
            format: replay::format::FormatVersion::default(),
            series,
            selected,
            seen: (0..len).map(|_| Seen::default()).collect(),
            identities: vec![None; len],
            owners: HashMap::new(),
            countries_from: None,
            country_keys: Vec::new(),
            counts_from: None,
            roster_from: None,
            roster_keys: Vec::new(),
            members: Vec::new(),
            owners_known: false,
            counts: vec![None; len],
            available: false,
        }
    }

    fn countries(&mut self, frame: &DecodedFrame) -> io::Result<bool> {
        let countries = frame.complete::<tables::Countries>()?;
        let links = frame.complete::<tables::CountryLinks>()?;
        let ids = [TableId::Countries, TableId::CountryLinks];
        if self.countries_from.as_ref().is_some_and(|previous| {
            ids.iter()
                .zip(previous)
                .all(|(id, seen)| seen.is(&frame.tables[id.index()]))
        }) {
            return Ok(false);
        }
        // Other country values change every hour; only identity facts change
        // this join. Compare those before rebuilding its maps.
        if self
            .countries_from
            .as_ref()
            .is_some_and(|previous| previous[1].is(&frame.tables[TableId::CountryLinks.index()]))
            && countries.len() == self.country_keys.len()
            && countries
                .iter()
                .zip(&self.country_keys)
                .all(|(row, (id, slot, tag))| {
                    (row.id, row.canonical_slot) == (*id, *slot) && row.tag == *tag
                })
        {
            self.countries_from = Some(ids.map(|id| frame.tables[id.index()].identity()));
            return Ok(false);
        }
        self.identities.fill(None);
        let tagged = tagged_countries(countries, links);
        let mut slots = HashMap::with_capacity(self.selected.len());
        for (tag, &index) in &self.selected {
            if let Some((country, slot)) =
                tagged.get(tag.as_bytes()).and_then(|lookup| lookup.found())
            {
                self.identities[index] = Some((slot, country.id));
                slots.insert(slot, index);
            }
        }
        self.owners = links
            .iter()
            .map(|link| (link.id, slots.get(&link.canonical_slot).copied()))
            .collect();
        self.country_keys = countries
            .iter()
            .map(|row| (row.id, row.canonical_slot, row.tag.clone()))
            .collect();
        self.countries_from = Some(ids.map(|id| frame.tables[id.index()].identity()));
        Ok(true)
    }

    /// The real hourly builder, also exercised by the nine-year scale fixture.
    fn observe(&mut self, frame: &DecodedFrame, defines: Defines) -> io::Result<()> {
        let hour = frame.hour;
        let countries = self.countries(frame);
        if countries.is_err() {
            self.identities.fill(None);
            self.countries_from = None;
        }
        for (state, identity) in self.seen.iter_mut().zip(&self.identities) {
            state.open &= state.identity.step(*identity, hour);
        }
        let ids = [
            TableId::Divisions,
            TableId::DivisionLandInputs,
            TableId::DivisionStatus,
        ];
        let read = ids
            .iter()
            .all(|id| slot_read(self.format, &frame.metrics, *id, hour));
        let divisions = frame.complete::<tables::Divisions>();
        let land = recorded_rows::<tables::DivisionLandInputs>(frame);
        let status = recorded_rows::<tables::DivisionStatus>(frame);
        let land_present = land_present(frame);
        self.available |= land_present;
        let valid = countries.is_ok()
            && divisions.is_ok()
            && land.is_ok()
            && status.is_ok()
            && land_present
            && read;
        let same = matches!(countries, Ok(false))
            && self.counts_from.as_ref().is_some_and(|previous| {
                ids.iter()
                    .zip(previous)
                    .all(|(id, seen)| seen.is(&frame.tables[id.index()]))
            });
        if !valid {
            self.counts.fill(None);
            self.counts_from = None;
        } else if !same {
            for (counts, identity) in self.counts.iter_mut().zip(&self.identities) {
                *counts = identity.map(|_| [0; 3]);
            }
            let divisions = divisions?;
            let same_roster = matches!(countries, Ok(false))
                && (self
                    .roster_from
                    .as_ref()
                    .is_some_and(|from| from.is(&frame.tables[TableId::Divisions.index()]))
                    || (divisions.len() == self.roster_keys.len()
                        && divisions
                            .iter()
                            .zip(&self.roster_keys)
                            .all(|(row, key)| (row.id, owner(row)) == *key)));
            if !same_roster {
                self.members.clear();
                self.owners_known = true;
                for division in divisions {
                    let Some(index) = self.owners.get(&owner(division)) else {
                        // An unresolved division might belong to any selected tag.
                        self.owners_known = false;
                        break;
                    };
                    if let Some(index) = *index {
                        self.members.push((division.id, index));
                    }
                }
                self.roster_keys = divisions.iter().map(|row| (row.id, owner(row))).collect();
            }
            self.roster_from = Some(frame.tables[TableId::Divisions.index()].identity());
            if !self.owners_known {
                self.counts.fill(None);
            } else {
                let land = land?;
                let status = status?;
                for &(handle, index) in &self.members {
                    let row = land
                        .binary_search_by_key(&handle, |row| row.division)
                        .ok()
                        .zip(
                            status
                                .binary_search_by_key(&handle, |row| row.division)
                                .ok(),
                        )
                        .and_then(|(l, s)| Supply::of(&status[s], &land[l]));
                    if let (Some(counts), Some(row)) = (&mut self.counts[index], row) {
                        let bucket = match defines.frame(&row) {
                            3 => 0,
                            1 | 2 => 1,
                            _ => 2,
                        };
                        counts[bucket] = counts[bucket].checked_add(1).ok_or_else(|| {
                            crate::invalid("Supply health division count overflow")
                        })?;
                    } else {
                        self.counts[index] = None;
                    }
                }
            }
            self.counts_from = Some(ids.map(|id| frame.tables[id.index()].identity()));
        }
        for ((series, seen), counts) in self.series.iter_mut().zip(&mut self.seen).zip(&self.counts)
        {
            let Some([no_supply, strained, in_supply]) = *counts else {
                seen.open = false;
                continue;
            };
            if seen.open
                && let Some(last) = series.runs.last_mut()
                && [last.no_supply, last.strained, last.in_supply]
                    == [no_supply, strained, in_supply]
            {
                last.last_hour = hour;
            } else {
                series.runs.push(SupplyHealthRun {
                    first_hour: hour,
                    last_hour: hour,
                    starts_segment: !seen.open,
                    no_supply,
                    strained,
                    in_supply,
                });
            }
            seen.open = true;
        }
        Ok(())
    }
}

fn reply_too_large() -> io::Error {
    io::Error::other(
        "The complete supply health reply exceeds MAX_PRODUCT_REPLY_BYTES; nothing was omitted",
    )
}

fn scan(
    replay: &Replay,
    branch: u64,
    tags: &[String],
    defines: Defines,
    cancelled: &AtomicBool,
) -> io::Result<Scan> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Divisions,
        TableId::DivisionLandInputs,
        TableId::DivisionStatus,
    ];
    let mut scan = Scan::new(tags);
    scan.format = replay.format;
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    scan.observe(decoded, defines)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(scan)
}

fn reply(branch_id: String, scan: Scan, warnings: Vec<String>) -> io::Result<SupplyHealthReply> {
    let reply = SupplyHealthReply {
        branch_id,
        available: scan.available,
        reason: (!scan.available).then(|| "DivisionLandInputs is not in this recording.".into()),
        missing_inputs: if scan.available {
            Vec::new()
        } else {
            vec!["DivisionLandInputs".into()]
        },
        series: scan.series,
        warnings,
    };
    ensure_reply(&reply)?;
    Ok(reply)
}

fn ensure_reply(reply: &SupplyHealthReply) -> io::Result<()> {
    if json_len(reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(reply_too_large());
    }
    Ok(())
}

#[derive(Default)]
struct Cache {
    branch: Option<u64>,
    defines: Option<Defines>,
    entries: HashMap<(u64, String), (bool, Arc<SupplyHealthSeries>)>,
    in_flight: HashMap<u64, Weak<Mutex<()>>>,
    content: Option<(Arc<installed_content::content::ResolvedContent>, Defines)>,
    inputs: HashMap<u64, bool>,
}

impl Cache {
    fn select_branch(&mut self, branch: u64) {
        self.branch = Some(branch);
        self.entries.retain(|(from, _), _| *from == branch);
        self.inputs.retain(|from, _| *from == branch);
        self.in_flight.retain(|_, slot| slot.strong_count() > 0);
    }

    fn flight(&mut self, branch: u64) -> Arc<Mutex<()>> {
        if let Some(slot) = self.in_flight.get(&branch).and_then(Weak::upgrade) {
            return slot;
        }
        let slot = Arc::new(Mutex::new(()));
        self.in_flight.insert(branch, Arc::downgrade(&slot));
        slot
    }
}

fn cached_scan(
    state: &Mutex<Cache>,
    branch: u64,
    tags: &[String],
    defines: Defines,
    cancelled: &AtomicBool,
    build: impl FnOnce(&[String]) -> io::Result<Scan>,
) -> io::Result<Scan> {
    let (slot, cached) = {
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        cache.select_branch(branch);
        if cache.defines != Some(defines) {
            cache.entries.clear();
            cache.defines = Some(defines);
        }
        let cached = tags
            .iter()
            .filter_map(|tag| {
                cache
                    .entries
                    .get(&(branch, tag.clone()))
                    .map(|entry| (tag.clone(), entry.clone()))
            })
            .collect::<HashMap<_, _>>();
        (cache.flight(branch), cached)
    };
    let mut result = Scan::new(tags);
    let mut cached = cached;
    if cached.len() != result.series.len() {
        // Waiting is cancellable; the cache mutex never covers a branch scan.
        let _flight = loop {
            check_cancel(cancelled)?;
            match slot.try_lock() {
                Ok(guard) => break guard,
                Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(Duration::from_millis(10))
                }
            }
        };
        {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            if cache.defines == Some(defines) {
                for tag in tags {
                    if let Some(entry) = cache.entries.get(&(branch, tag.clone())) {
                        cached.insert(tag.clone(), entry.clone());
                    }
                }
            }
        }
        let missing: Vec<String> = result
            .series
            .iter()
            .filter(|series| !cached.contains_key(&series.tag))
            .map(|series| series.tag.clone())
            .collect();
        if !missing.is_empty() {
            let built = build(&missing)?;
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            for series in built.series {
                let tag = series.tag.clone();
                let entry = (built.available, Arc::new(series));
                if cache.branch == Some(branch) && cache.defines == Some(defines) {
                    cache.entries.insert((branch, tag.clone()), entry.clone());
                }
                cached.insert(tag, entry);
            }
        }
    }
    for series in &mut result.series {
        if let Some((available, found)) = cached.get(&series.tag) {
            result.available |= *available;
            *series = (**found).clone();
        }
    }
    check_cancel(cancelled)?;
    Ok(result)
}

fn land_present(frame: &DecodedFrame) -> bool {
    !matches!(
        &*frame.tables[TableId::DivisionLandInputs.index()],
        replay::frame::TableView::Unavailable { .. }
    )
}

fn has_inputs(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<bool> {
    let selected = [TableId::Campaign, TableId::DivisionLandInputs];
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                check_cancel(cancelled)?;
                let Ok(frames) = frames else {
                    continue;
                };
                let mut decoder = FrameDecoder::for_tables(&selected);
                for frame in frames {
                    check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    if land_present(decoded) {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        },
    )
}

impl Product for SupplyHealth {
    const KEY: &'static str = "supply-health";
    const CONTENT_DEPENDENT: bool = true;
    type Params = SupplyHealthParams;
    type Reply = SupplyHealthReply;

    fn types() -> Vec<String> {
        vec![
            SupplyHealthParams::decl(),
            SupplyHealthRun::decl(),
            SupplyHealthSeries::decl(),
            SupplyHealthReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        if params.tags.is_empty() {
            return Err("Choose countries for supply health");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let replay = source.replay_ref();
        if !replay.branches.iter().any(|b| b.id == branch) {
            return Err(crate::invalid(
                "Supply health branch is not in this recording",
            ));
        }
        let state = source.state::<Cache>();
        let inputs = state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .inputs
            .get(&branch)
            .copied();
        let inputs = if let Some(inputs) = inputs {
            inputs
        } else {
            let inputs = has_inputs(replay, branch, cancelled)?;
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache.select_branch(branch);
            cache.inputs.insert(branch, inputs);
            inputs
        };
        if !inputs {
            return reply(
                params.branch_id.clone(),
                Scan::new(&params.tags),
                Vec::new(),
            );
        }
        let mut warnings = Vec::new();
        let options = context.content()?;
        content::check_replay(replay, options.allow_unverified, &mut warnings)?;
        let content = source.content(&options.options)?;
        let cached = state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .content
            .clone();
        let defines = if let Some((from, defines)) = cached
            && Arc::ptr_eq(&from, &content)
        {
            defines
        } else {
            let defines = content_defines(&content.layers)?;
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache.content = Some((content, defines));
            defines
        };
        let history =
            super::country_history::branch_histories(source, branch, false, &[], cancelled)?;
        let selected = super::country_history::selected_identities(&history, &params.tags);
        let tags = super::country_history::selected_aliases(&history, &params.tags);
        let mut built = cached_scan(&state, branch, &tags, defines, cancelled, |missing| {
            scan(replay, branch, missing, defines, cancelled)
        })?;
        for series in &mut built.series {
            let identities: Vec<_> = history
                .countries
                .get(&series.tag)
                .map_or(&[][..], |country| country.identities.as_slice())
                .iter()
                .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
                .cloned()
                .collect();
            let spans: Vec<_> = identities
                .iter()
                .map(|identity| super::country_groups::HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                })
                .collect();
            let mut runs = Vec::new();
            crate::ranking::for_each_piece(
                &series.runs,
                &spans,
                |run| (run.first_hour, run.last_hour),
                cancelled,
                |run, first_hour, last_hour| {
                    let mut piece = run.clone();
                    piece.first_hour = first_hour;
                    piece.last_hour = last_hour;
                    runs.push(piece);
                    Ok(())
                },
            )?;
            series.runs = runs;
            series.identities = Some(identities);
        }
        reply(params.branch_id.clone(), built, warnings)
    }
}

#[cfg(test)]
mod tests;
