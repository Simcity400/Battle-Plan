//! Division supply-icon states, counted at their hourly reads.
//!
//! Low supply is false during naval transfer, otherwise
//! effective supply < NMilitary.LOW_SUPPLY. Stored signed operands hold the
//! received fixed5 fraction, maximum grace and remaining grace.
//! Maximum grace starts with SUPPLY_GRACE (integer hours) * 100000; the hourly
//! division update passes remaining grace and its companion to
//! the grace updater. We read stored values, never recreate
//! their update or make them fresher. DivisionStatus records these exact
//! signed i64 operands (supply_ratio, supply_grace_maximum/remaining) and
//! DivisionLandInputs the naval-transfer presence, both from the same hour's
//! authenticated division seed, keyed by full u64 handle,
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
//! conversion is shared by installed_content_core::defines::fixed5. Every value resolves
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
//! Selected replies read prepared runs and summaries, with progressive samples
//! beyond accepted coverage. The complete selected reply refuses only past
//! MAX_PRODUCT_REPLY_BYTES.

use dashboard_base::{
    content,
    products::{MAX_PRODUCT_REPLY_BYTES, Product},
    source::Source,
};
use dashboard_core::{config::LoadContext, protocol::validate_tag, transport::json_len};
use dashboard_text::metadata::{IdentityTrack, recorded_rows, slot_read, tagged_countries};
use std::collections::BTreeMap;

#[cfg(any(test, feature = "test-support"))]
use dashboard_text::{metadata::check_cancel, metadata::validate_campaign};
#[cfg(any(test, feature = "test-support"))]
use replay::container::Replay;
#[cfg(test)]
use replay_data::rows::DivisionLandInputsRow;
use replay_data::{
    rows::DivisionRow, rows::DivisionStatusRow, rows::Recorded, rows::TableId, rows::tables,
};
#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::FrameDecoder;
use replay_frame::{frame::DecodedFrame, frame::ViewIdentity};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "test-support"))]
use std::{sync::Arc, sync::Mutex, sync::Weak, time::Duration};
use ts_rs::TS;

pub struct SupplyHealth;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SupplyHealthParams {
    pub branch_id: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
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

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SupplyHealthSeries {
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<dashboard_base::products::country_history::CountryIdentityRun>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub summaries: Option<Vec<SupplyHealthDisplay>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SupplyHealthDisplay {
    pub tag: String,
    pub values: Vec<dashboard_base::prepared_chart::ChartSummary>,
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

type CountryCounts = BTreeMap<usize, Option<[u32; 3]>>;

impl Supply {
    fn parts(status: &DivisionStatusRow, naval_transfer: bool) -> Option<Self> {
        let (Recorded::Value(maximum_grace), Recorded::Value(remaining_grace)) =
            (status.supply_grace_maximum, status.supply_grace_remaining)
        else {
            return None;
        };
        Some(Self {
            received: status.supply_ratio,
            maximum_grace,
            remaining_grace,
            naval_transfer,
        })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
struct Defines {
    grace: i32,
    low: i64,
    very_low_icon: i64,
    low_icon: i64,
}

pub fn check_content(layers: &installed_content_core::ResolvedLayers) -> io::Result<()> {
    content_defines(layers).map(|_| ())
}

fn content_defines(layers: &installed_content_core::ResolvedLayers) -> io::Result<Defines> {
    layers
        .derived::<dashboard_text::catalogue::CachedResult<Defines>>()
        .get_or_init(|| Defines::read(&installed_content_core::defines::read(layers)?))
        .map(|defines| *defines)
}

impl Defines {
    fn read(defines: &installed_content_core::defines::Defines) -> io::Result<Self> {
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
            return Err(dashboard_core::invalid(format!(
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

use dashboard_base::prepared::frame_changes::{FrameChanges, field_mask};
use std::sync::LazyLock;
static COUNTRY_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::Countries, &["canonical_slot", "tag"]));
static DIVISION_OWNER_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::Divisions, &["roster_owner", "expeditionary_owner"]));
static TRANSFER_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::DivisionLandInputs, &["naval_transfer"]));
static SUPPLY_FIELDS: LazyLock<u64> = LazyLock::new(|| {
    field_mask(
        TableId::DivisionStatus,
        &[
            "supply_ratio",
            "supply_grace_maximum",
            "supply_grace_remaining",
        ],
    )
});
fn identity_rows_kept(
    changes: &FrameChanges,
    table: TableId,
    count: usize,
    previous: usize,
    fields: &u64,
) -> bool {
    count == previous
        && changes.rows(table).is_some_and(|(changed, deleted)| {
            deleted.is_empty() && changed.iter().all(|row| row.mask & fields == 0)
        })
}

fn owner(row: &DivisionRow) -> u32 {
    if (row.expeditionary_owner as i32) > 0 {
        row.expeditionary_owner
    } else {
        row.roster_owner
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Seen {
    identity: IdentityTrack,
    open: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Scan {
    /// The recording's format, which dates its daily tables
    /// ([`dashboard_text::metadata::slot_read`]).
    #[serde(skip)]
    format: replay_data::format::FormatVersion,
    series: Vec<SupplyHealthSeries>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    selected: HashMap<String, usize>,
    seen: Vec<Seen>,
    #[serde(skip)]
    identities: Vec<Option<(u32, u32)>>,
    #[serde(skip)]
    owners: HashMap<u32, Option<usize>>,
    #[serde(skip)]
    countries_from: Option<[ViewIdentity; 2]>,
    #[serde(skip)]
    country_keys: Option<dashboard_text::metadata::CountryKeys>,
    #[serde(skip)]
    country_rows: Option<usize>,
    #[serde(skip)]
    roster_from: Option<ViewIdentity>,
    #[serde(skip)]
    roster_keys: Vec<(u64, u32)>,
    #[serde(skip)]
    members: Vec<(u64, usize)>,
    #[serde(skip)]
    owners_known: bool,
    #[serde(skip)]
    member_owners: HashMap<u64, usize>,
    #[serde(skip)]
    buckets: HashMap<u64, Option<usize>>,
    #[serde(skip)]
    totals: BTreeMap<usize, ([u32; 3], usize)>,
    #[serde(skip)]
    buckets_ready: bool,
    #[cfg(test)]
    #[serde(skip)]
    division_visits: usize,
    #[cfg(test)]
    #[serde(skip)]
    total_visits: usize,
    #[serde(skip)]
    transfers: HashMap<u64, bool>,
    #[serde(skip)]
    status_inputs: HashMap<u64, Option<Supply>>,
    #[serde(skip)]
    populated: std::collections::BTreeSet<usize>,
    #[serde(skip)]
    observed: Vec<usize>,
    #[serde(skip)]
    counts: Vec<Option<[u32; 3]>>,
    available: bool,
    frontier: Option<i32>,
    #[serde(skip)]
    current: Vec<usize>,
    #[serde(skip)]
    departed: Vec<usize>,
    #[serde(skip)]
    valid: Option<bool>,
    #[serde(skip)]
    rejoined: bool,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}

impl Scan {
    fn add_tags<'a>(&mut self, tags: impl IntoIterator<Item = &'a String>) {
        for tag in tags {
            if self.selected.contains_key(tag) {
                continue;
            }
            self.selected.insert(tag.clone(), self.series.len());
            self.series.push(SupplyHealthSeries {
                tag: tag.clone(),
                identities: None,
                runs: Vec::new(),
            });
            self.seen.push(Seen::default());
            self.identities.push(None);
            self.counts.push(None);
            self.countries_from = None;
        }
    }
    fn new(tags: &[String]) -> Self {
        let mut scan = Self::default();
        scan.add_tags(tags);
        scan
    }

    fn countries(&mut self, frame: &DecodedFrame) -> io::Result<bool> {
        self.country_changes(frame, None)
    }
    fn country_changes(
        &mut self,
        frame: &DecodedFrame,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<bool> {
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
            && (changes.is_some_and(|changes| {
                identity_rows_kept(
                    changes,
                    TableId::Countries,
                    countries.len(),
                    self.country_rows.unwrap_or(usize::MAX),
                    &COUNTRY_FIELDS,
                )
            }) || self.country_keys.as_ref().is_some_and(|keys| {
                #[cfg(test)]
                {
                    self.work.row_visits += countries.len();
                }
                keys.same(countries)
            }))
        {
            self.countries_from = Some(ids.map(|id| frame.tables[id.index()].identity()));
            return Ok(false);
        }
        self.identities.resize(self.series.len(), None);
        self.counts.resize(self.series.len(), None);
        let tagged = tagged_countries(countries, links);
        self.seal();
        let previous = std::mem::take(&mut self.current);
        let mut present = std::collections::HashSet::new();
        let mut slots = HashMap::new();
        for (tag, lookup) in tagged {
            #[cfg(test)]
            {
                self.work.row_visits += 1;
            }
            let Some(&index) = std::str::from_utf8(tag)
                .ok()
                .and_then(|tag| self.selected.get(tag))
            else {
                continue;
            };
            present.insert(index);
            self.current.push(index);
            self.identities[index] = None;
            if let Some((country, slot)) = lookup.found() {
                self.identities[index] = Some((slot, country.id));
                slots.insert(slot, index);
            }
        }
        self.departed = previous
            .into_iter()
            .filter(|index| !present.contains(index))
            .collect();
        for &index in &self.departed {
            self.identities[index] = None;
            self.counts[index] = None;
        }
        self.owners = links
            .iter()
            .map(|link| (link.id, slots.get(&link.canonical_slot).copied()))
            .collect();
        self.country_keys = Some(dashboard_text::metadata::CountryKeys::read(countries));
        self.country_rows = Some(countries.len());
        self.countries_from = Some(ids.map(|id| frame.tables[id.index()].identity()));
        Ok(true)
    }

    /// The real hourly builder, also exercised by the nine-year scale fixture.
    fn observe(&mut self, frame: &DecodedFrame, defines: Defines) -> io::Result<()> {
        self.observe_changes(frame, defines, None).map(|_| ())
    }

    fn seal(&mut self) {
        let Some(hour) = self.frontier else {
            return;
        };
        for &index in &self.current {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            let seen = &mut self.seen[index];
            seen.identity.advance(hour);
            if seen.open
                && let Some(run) = self.series[index].runs.last_mut()
            {
                run.last_hour = hour;
            }
        }
    }

    fn count_divisions(
        &mut self,
        land: &[replay_data::rows::DivisionLandInputsRow],
        status: &[DivisionStatusRow],
        defines: Defines,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
        continuous_roster: bool,
    ) -> io::Result<(bool, CountryCounts)> {
        let deltas = changes.and_then(|changes| {
            Some((
                changes.rows(TableId::DivisionLandInputs)?,
                changes.rows(TableId::DivisionStatus)?,
            ))
        });
        let partial =
            continuous_roster && self.buckets_ready && self.owners_known && deltas.is_some();
        let dirty: Vec<u64> = if partial {
            let (land_delta, status_delta) = deltas.expect("delta counts");
            let mut dirty = std::collections::BTreeSet::new();
            for &key in land_delta.1 {
                self.transfers.remove(&(key as u64));
                dirty.insert(key as u64);
            }
            for change in land_delta.0 {
                if change.mask & *TRANSFER_FIELDS == 0
                    && self.transfers.contains_key(&(change.key as u64))
                {
                    continue;
                }
                let at = land
                    .binary_search_by_key(&(change.key as u64), |row| row.division)
                    .map_err(|_| dashboard_core::invalid("Missing changed supply land row"))?;
                self.transfers
                    .insert(land[at].division, land[at].naval_transfer);
                dirty.insert(land[at].division);
            }
            for &key in status_delta.1 {
                self.status_inputs.remove(&(key as u64));
                dirty.insert(key as u64);
            }
            for change in status_delta.0 {
                if change.mask & *SUPPLY_FIELDS == 0
                    && self.status_inputs.contains_key(&(change.key as u64))
                {
                    continue;
                }
                let at = status
                    .binary_search_by_key(&(change.key as u64), |row| row.division)
                    .map_err(|_| dashboard_core::invalid("Missing changed supply status row"))?;
                self.status_inputs
                    .insert(status[at].division, Supply::parts(&status[at], false));
                dirty.insert(status[at].division);
            }
            dirty.into_iter().collect()
        } else {
            if !self.buckets_ready
                || changes
                    .is_none_or(|changes| changes.rows_changed[TableId::DivisionLandInputs.index()])
            {
                #[cfg(test)]
                {
                    self.work.row_visits += land.len();
                }
                self.transfers = land
                    .iter()
                    .map(|row| (row.division, row.naval_transfer))
                    .collect();
            }
            if !self.buckets_ready
                || changes
                    .is_none_or(|changes| changes.rows_changed[TableId::DivisionStatus.index()])
            {
                #[cfg(test)]
                {
                    self.work.row_visits += status.len();
                }
                self.status_inputs = status
                    .iter()
                    .map(|row| (row.division, Supply::parts(row, false)))
                    .collect();
            }
            self.member_owners = self.members.iter().copied().collect();
            self.totals.clear();
            self.buckets.clear();
            self.members.iter().map(|&(handle, _)| handle).collect()
        };
        let mut touched = std::collections::BTreeSet::new();
        if self.owners_known {
            for handle in dirty {
                let Some(&index) = self.member_owners.get(&handle) else {
                    continue;
                };
                #[cfg(test)]
                {
                    self.division_visits += 1;
                }
                let total = self.totals.entry(index).or_default();
                if partial {
                    touched.insert(index);
                }
                match self.buckets.get(&handle) {
                    Some(Some(bucket)) => total.0[*bucket] -= 1,
                    Some(None) => total.1 -= 1,
                    None => {}
                }
                let bucket = self
                    .status_inputs
                    .get(&handle)
                    .copied()
                    .flatten()
                    .zip(self.transfers.get(&handle))
                    .map(|(mut row, transfer)| {
                        row.naval_transfer = *transfer;
                        match defines.frame(&row) {
                            3 => 0,
                            1 | 2 => 1,
                            _ => 2,
                        }
                    });
                if let Some(bucket) = bucket {
                    total.0[bucket] = total.0[bucket].checked_add(1).ok_or_else(|| {
                        dashboard_core::invalid("Supply health division count overflow")
                    })?;
                } else {
                    total.1 += 1;
                }
                self.buckets.insert(handle, bucket);
            }
        }
        self.buckets_ready = self.owners_known;
        let indices: Vec<_> = if partial {
            touched.into_iter().collect()
        } else {
            self.totals.keys().copied().collect()
        };
        Ok((
            partial,
            indices
                .into_iter()
                .map(|index| {
                    #[cfg(test)]
                    {
                        self.total_visits += 1;
                    }
                    let (counts, unknown) = self.totals[&index];
                    (
                        index,
                        (unknown == 0)
                            .then_some(self.identities[index])
                            .flatten()
                            .map(|_| counts),
                    )
                })
                .collect(),
        ))
    }

    fn observe_changes(
        &mut self,
        frame: &DecodedFrame,
        defines: Defines,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<bool> {
        let cpu = ::replay_frame::cpu::scope_phase("supply:observe_apply");
        let hour = frame.hour;
        self.identities.resize(self.series.len(), None);
        self.counts.resize(self.series.len(), None);
        self.departed.clear();
        self.observed.clear();
        // Publication seals these endpoints; restore rebuilds only the current
        // open roster, before its native join is refreshed from the next frame.
        if !self.rejoined && self.frontier.is_some() {
            self.current = self
                .seen
                .iter()
                .enumerate()
                .filter_map(|(index, seen)| {
                    (seen.open
                        && self.series[index]
                            .runs
                            .last()
                            .is_some_and(|run| Some(run.last_hour) == self.frontier))
                    .then_some(index)
                })
                .collect();
        }
        self.rejoined = true;
        let continuous = self
            .frontier
            .is_some_and(|last| last.checked_add(1) == Some(hour));
        // Materialise only at a change, before a changed country roster closes
        // holders. Unchanged frames touch the authenticated frontier alone.
        let mut relevant = changes.is_none_or(|changes| {
            [
                TableId::Divisions,
                TableId::DivisionLandInputs,
                TableId::DivisionStatus,
            ]
            .iter()
            .any(|id| changes.rows_changed[id.index()])
        });
        if !continuous {
            self.seal();
        }
        let countries = self.country_changes(frame, changes);
        relevant |= !matches!(countries, Ok(false));
        if countries.is_err() {
            self.seal();
            self.departed = std::mem::take(&mut self.current);
            for &index in &self.departed {
                self.identities[index] = None;
                self.counts[index] = None;
            }
            self.countries_from = None;
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
        if !relevant && continuous && self.valid == Some(valid) {
            self.frontier = Some(hour);
            self.departed.clear();
            return Ok(false);
        }
        let mut full = !continuous
            || !matches!(countries, Ok(false))
            || self.valid != Some(valid)
            || changes.is_none();
        self.valid = Some(valid);
        let mut next = CountryCounts::new();
        let mut partial = false;
        if valid {
            let divisions = divisions?;
            let same_roster = matches!(countries, Ok(false))
                && (self
                    .roster_from
                    .as_ref()
                    .is_some_and(|view| view.is(&frame.tables[TableId::Divisions.index()]))
                    || changes.is_some_and(|changes| {
                        identity_rows_kept(
                            changes,
                            TableId::Divisions,
                            divisions.len(),
                            self.roster_keys.len(),
                            &DIVISION_OWNER_FIELDS,
                        )
                    })
                    || divisions.len() == self.roster_keys.len()
                        && divisions
                            .iter()
                            .zip(&self.roster_keys)
                            .all(|(row, key)| (row.id, owner(row)) == *key));
            if !same_roster {
                let before = self.owners_known;
                self.members.clear();
                self.owners_known = true;
                for division in divisions {
                    #[cfg(test)]
                    {
                        self.work.row_visits += 1;
                    }
                    let Some(index) = self.owners.get(&owner(division)) else {
                        self.owners_known = false;
                        break;
                    };
                    if let Some(index) = *index {
                        self.members.push((division.id, index));
                    }
                }
                full |= before != self.owners_known;
                self.roster_keys = divisions.iter().map(|row| (row.id, owner(row))).collect();
            }
            self.roster_from = Some(frame.tables[TableId::Divisions.index()].identity());
            drop(cpu);
            let _cpu = ::replay_frame::cpu::scope_phase("supply:division_counts");
            (partial, next) =
                self.count_divisions(land?, status?, defines, changes, !full && same_roster)?;
        } else {
            self.buckets_ready = false;
            drop(cpu);
        }
        let _cpu = ::replay_frame::cpu::scope_phase("supply:projection_series");
        let candidates: Vec<_> = if partial {
            next.keys().copied().collect()
        } else {
            let populated = next.keys().copied().collect();
            let candidates = if full {
                self.current.clone()
            } else {
                self.populated.union(&populated).copied().collect()
            };
            self.populated = populated;
            candidates
        };
        for index in candidates {
            let after = next.remove(&index).unwrap_or_else(|| {
                (valid && self.owners_known)
                    .then_some(self.identities[index])
                    .flatten()
                    .map(|_| [0; 3])
            });
            if !full && self.counts[index] == after {
                continue;
            }
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            let series = &mut self.series[index];
            let seen = &mut self.seen[index];
            if continuous && let Some(frontier) = self.frontier {
                seen.identity.advance(frontier);
                if seen.open
                    && let Some(run) = series.runs.last_mut()
                {
                    run.last_hour = frontier;
                }
            }
            seen.open &= seen.identity.step(self.identities[index], hour);
            self.counts[index] = after;
            self.observed.push(index);
            let Some([no_supply, strained, in_supply]) = after else {
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
        for &index in &self.departed {
            self.seen[index].open = false;
        }
        self.frontier = Some(hour);
        Ok(true)
    }
}

fn reply_too_large() -> io::Error {
    io::Error::other(
        "The complete supply health reply exceeds MAX_PRODUCT_REPLY_BYTES; nothing was omitted",
    )
}

#[cfg(any(test, feature = "test-support"))]
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
    dashboard_core::blocks::scan_blocks(
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
        summaries: None,
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
#[cfg(any(test, feature = "test-support"))]
struct Cache {
    branch: Option<u64>,
    defines: Option<Defines>,
    entries: HashMap<(u64, String), (bool, Arc<SupplyHealthSeries>)>,
    in_flight: HashMap<u64, Weak<Mutex<()>>>,
    content: Option<(Arc<installed_content_core::content::ResolvedContent>, Defines)>,
    inputs: HashMap<u64, bool>,
}

#[cfg(any(test, feature = "test-support"))]
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

#[cfg(any(test, feature = "test-support"))]
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
        replay_frame::frame::TableView::Unavailable { .. }
    )
}

#[cfg(any(test, feature = "test-support"))]
fn has_inputs(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<bool> {
    let selected = [TableId::Campaign, TableId::DivisionLandInputs];
    dashboard_core::blocks::scan_blocks(
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
            SupplyHealthDisplay::decl(),
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

    #[cfg(any(test, feature = "test-support"))]
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
            return Err(dashboard_core::invalid(
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
        let history = dashboard_base::products::country_history::branch_histories(
            source,
            branch,
            false,
            &[],
            cancelled,
        )?;
        let selected =
            dashboard_base::products::country_history::selected_identities(&history, &params.tags);
        let tags =
            dashboard_base::products::country_history::selected_aliases(&history, &params.tags);
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
                .map(
                    |identity| dashboard_base::products::country_groups::HourSpan {
                        first_hour: identity.first_hour,
                        last_hour: identity.last_hour,
                    },
                )
                .collect();
            let mut runs = Vec::new();
            dashboard_base::ranking::for_each_piece(
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

pub mod prepared;
#[cfg(test)]
pub mod tests;
