//! One country's fuel over a branch, for the Fuel widget: the tank's level and
//! capacity at each read, and the fuel gained and burned in the day before it.
//! The applied-award day (`CountryFuel.award_day`, the collector's sums of 24
//! authenticated updates) supersedes ring sums when it matches the supplying
//! read and country identity. Recordings before it, whose update-by-update
//! windows are retired, and unknown days retain the ring/ownership fallback
//! below; they never infer source splits.
//!
//! Every value is a recorded `CountryFuel` field, kept exact: the level and
//! capacity (`current_raw`, `maximum_raw`) are quantities in /32768 units, and
//! so is what was burned; what was gained is fixed5. The game's hourly fuel
//! update books each hour into its 24-slot history ring:
//!
//! - `fuel_gain` is the fixed5 amount it adds from base gain, states, oil and a
//!   negative flat cost, before converting it to a quantity. Lend-lease fuel
//!   received is added to the tank but not to this entry; only the latest hour's
//!   amount is recorded (`lend_lease_gain_raw`), so a day's lend-lease total is not
//!   known from that prefix and each legacy read says only whether it was
//!   arriving then. Applied suffixes retain every actual hourly LL amount.
//! - `consumed` is everything it takes out: the flat cost and lend-lease fuel
//!   sent (both also booked as `other`, so `other` is part of `consumed`), and
//!   what the three consumers received. It equals `other` plus the sum of the
//!   converted `received` values.
//! - The tank is then clamped to zero and to its capacity: what does not fit is
//!   discarded.
//!
//! So a day's gained is the recorded ring sum `ring_gain_raw` and its burned
//! is `ring_consumed_raw`. Conversion rounding and tank clamps can make these
//! differ from the change in stored level.
//!
//! The game updates a country's fuel only while it owns a state. A country
//! without states keeps its last ring and level; its repeated ring values
//! therefore do not describe new fuel gain.
//!
//! Which recorded hours govern a read's 24 updates follows the hourly tick
//! order. The collector samples at the hourly-update entry
//! (`SampleBoundary::EndOfHour`): frame h is settled state at the end of hour h.
//! The tick then advances to h + 1 before running that hour's fuel update,
//! and the next sample follows it. The update booked in hour k ran after
//! sample k - 1 and before sample k; sample k - 1 is the last recorded state
//! it saw. Other changes before the fuel update are not recorded. A read at
//! sample r holds updates r - 23 ..= r, and their governing ownership was last
//! recorded at r - 24 ..= r - 1. The read's own sample governs the next update.
//!
//! A read's day is shown only when all 24 of those samples were recorded and
//! each shows the same country identity owning a state (`States`). A sample that
//! shows it holding none, or another country under the tag, leaves the day out
//! as not updated. A sample the recording lacks (a gap, or hours before the
//! branch), or holds without the tables to tell (`States`, `Countries` or
//! `CountryLinks` incomplete, or links that cannot resolve the tag's country, or
//! the owner of a state when none resolves to the country), leaves it out as
//! ownership not recorded: a
//! country that lost its last state in hours nobody saw stopped updating then,
//! so its ring still holds older updates and its sums would count them again,
//! and a frozen ring cannot be told from a live one. A sample without a state
//! outweighs a missing one.
//!
//! The sums cover the 24 hourly updates before a read, not a calendar day.
//! Reads in the fuel table's daily slot are 24 hours apart and tile time
//! exactly; a read at another hour (a branch start, a load, a gap or a
//! refresh) overlaps them. A read's day is kept only when it overlaps no other
//! kept day: every read in the slot, then any other read in hour order that is
//! at least 24 hours from each read kept, so no update is counted twice.
//!
//! A read is an hour whose frame timed a walk of any table of the fuel table's
//! slot (`recorder::oracle::last_read_hours`). Values start only at a read,
//! carry over the following hours of the same country identity, and end at a
//! gap, an incomplete table or a change of identity.

use dashboard_base::products::{CountryParams, Product};
use dashboard_base::source::Source;
use dashboard_core::config::LoadContext;
use dashboard_text::metadata::{link_slot, slot_read, tagged_countries, validate_campaign};
use engine_fuel::{
    ccountry_fuel_status::HISTORY_RING_ENTRIES, ccountry_fuel_status::MODIFIER_SCALE,
    ccountry_fuel_status::QUANTITY_SCALE,
};
use replay_data::{rows::CountryFuelRow, rows::FuelAwardDay, rows::TableId, rows::tables};
use replay_frame::frame::FrameDecoder;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;
#[cfg(any(test, feature = "test-support"))]
use {dashboard_text::metadata::IdentityTrack, replay_data::cadence, std::collections::VecDeque};

pub struct CountryFuel;
#[cfg(any(test, feature = "test-support"))]
pub mod oracle;
#[cfg(any(test, feature = "test-support"))]
use oracle::scanned;
#[cfg(test)]
use oracle::scan;
mod detail;
pub mod emit;
pub mod facts;
pub mod query;
pub mod served;
pub use detail::{CountryFuelAwardHistory, FuelAwardHistoryReply};
mod demand;
pub use demand::{
    CountryFuelConsumption, CountryFuelDemand, CountryFuelDemandHistory, CountryFuelDemandStack,
    FuelConsumptionReply, FuelDemandHistoryReply, FuelDemandReply, FuelDemandStackReply,
};

/// Hours of fuel updates one read's ring sums cover: the ring's slots, one per
/// hourly update.
const DAY_HOURS: i32 = HISTORY_RING_ENTRIES as i32;
/// The engine's scales as the reply sends them.
const QUANTITY_UNITS: u32 = QUANTITY_SCALE as u32;
const GAIN_UNITS: u32 = MODIFIER_SCALE as u32;
const _: () = assert!(
    QUANTITY_UNITS as i64 == QUANTITY_SCALE
        && GAIN_UNITS as i64 == MODIFIER_SCALE
        && DAY_HOURS as u32 == HISTORY_RING_ENTRIES
);

/// The fuel updates booked in the day before a read.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelDay {
    /// Own fuel gain, fixed5; applied updates when known, otherwise ring sum.
    pub gained_raw: String,
    /// Fuel they took out, in [`FuelReply::quantity_scale`] units
    /// (`ring_consumed_raw`), `ring_other_raw` included.
    pub burned_raw: String,
    /// Consumer receipts, fixed5; per-hour quantity conversions cannot be
    /// reconstructed from these sums.
    pub received_raw: [String; 3],
    /// Flat cost and fuel sent by lend-lease, in quantity units; already in burned.
    pub other_raw: String,
    /// Actual applied updates, independent of subsequently refreshed rates.
    pub awards: Option<FuelAwardTotals>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelAwardPart {
    /// Exact receipt-time grouping operands, independent of viewer language.
    pub key: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub kind: u32,
    pub reference: String,
    pub owner: String,
    pub host: String,
    /// Fixed5 country fuel allocated to this recipient; not unit-local burn.
    pub amount_raw: String,
    /// Retained game-language bytes at the write. No later/name-key backfill.
    pub recorded_name: Option<String>,
    pub name_reason: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelAwardTotals {
    /// Base, state-source modifiers, oil and negative flat cost; fixed5.
    pub gains_raw: [String; 4],
    pub own_gain_raw: String,
    /// Sum of the native per-hour own-gain conversions, in quantity units.
    pub own_gain_quantity_raw: String,
    /// Incoming LL, outgoing LL, positive flat cost, total other; Q15.
    pub transfers_raw: [String; 4],
    pub consumed_raw: String,
    pub received_raw: [String; 3],
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelAwards {
    pub totals: FuelAwardTotals,
    pub recipients: Vec<FuelAwardPart>,
}

/// [`FuelAwardTotals`] as exact integers; only replies format them.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct AwardSums {
    gains: [i128; 4],
    own_gain: i128,
    own_gain_quantity: i128,
    transfers: [i128; 4],
    consumed: i128,
    received: [i128; 3],
}

impl From<&AwardSums> for FuelAwardTotals {
    fn from(sums: &AwardSums) -> Self {
        Self {
            gains_raw: sums.gains.map(|n| n.to_string()),
            own_gain_raw: sums.own_gain.to_string(),
            own_gain_quantity_raw: sums.own_gain_quantity.to_string(),
            transfers_raw: sums.transfers.map(|n| n.to_string()),
            consumed_raw: sums.consumed.to_string(),
            received_raw: sums.received.map(|n| n.to_string()),
        }
    }
}

/// The supplying read's summed day of applied updates
/// (`CountryFuel.award_day`): complete, of this read's country identity,
/// observed since before its first update and ending at the read's hour.
fn award_day(row: &CountryFuelRow, hour: i32) -> Result<&FuelAwardDay, String> {
    let replay_data::rows::Recorded::Value(day) = &row.award_day else {
        return Err("Applied fuel source and recipient history not recorded".into());
    };
    if !day.reason.is_empty() {
        return Err(String::from_utf8_lossy(&day.reason).into_owned());
    }
    if !day.continuity_start_hour.is_some_and(|start| {
        hour.checked_sub(DAY_HOURS - 1)
            .is_some_and(|first| start <= first)
    }) {
        return Err(
            "Applied fuel observation continuity boundary missing or later than this window".into(),
        );
    }
    if day.last_hour != hour {
        return Err("Applied fuel window does not match this supplying read".into());
    }
    Ok(day)
}

fn awards_of(row: &CountryFuelRow, hour: i32) -> Result<AwardSums, String> {
    let day = award_day(row, hour)?;
    Ok(AwardSums {
        gains: day.gains_raw.map(i128::from),
        own_gain: i128::from(day.own_gain_raw),
        own_gain_quantity: i128::from(day.own_gain_quantity_raw),
        transfers: day.transfers_raw.map(i128::from),
        consumed: i128::from(day.consumed_raw),
        received: day.received_raw.map(i128::from),
    })
}

/// The read's recipients, in the order of their identity operands. Each
/// recorded recipient already gathers the day's receipts of identical native
/// identity, owner, host and retained name operands, so changed retained text
/// is a separate row, with no generated-name backfill.
fn recipients_of(
    row: &CountryFuelRow,
    cancelled: &AtomicBool,
    mut format_name: impl FnMut(&[u8]) -> Result<String, String>,
) -> io::Result<Vec<FuelAwardPart>> {
    let replay_data::rows::Recorded::Value(day) = &row.award_day else {
        return Err(dashboard_core::invalid("Fuel awards not recorded"));
    };
    let mut rows = engine_allocation::vec(day.recipients.len())?;
    for part in &day.recipients {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if part.kind > 2 {
            return Err(dashboard_core::invalid("Fuel recipient kind unsupported"));
        }
        let (kind, reference, owner, host, name, why) = (
            part.kind,
            part.reference,
            part.owner,
            part.host,
            part.name.as_deref(),
            part.name_reason.as_slice(),
        );
        let (first_hour, last_hour, amount) = (part.first_hour, part.last_hour, part.amount_raw);
        let parsed = name.map(&mut format_name).transpose();
        let (recorded_name, name_reason) = match parsed {
            Ok(name) => (
                name,
                (!why.is_empty()).then(|| String::from_utf8_lossy(why).into_owned()),
            ),
            Err(why) => (
                None,
                Some(format!("Retained native fuel recipient name: {why}")),
            ),
        };
        let name_reason =
            name_reason.or_else(|| {
                (kind==1).then(||
            "The native recipient is a base/country container; no wing receipt is recorded".into())
            });
        rows.push(FuelAwardPart {
            key: recipient_key(kind, reference, owner, host, name, why)?,
            first_hour,
            last_hour,
            kind,
            reference: reference.to_string(),
            owner: owner.to_string(),
            host: host.to_string(),
            amount_raw: amount.to_string(),
            recorded_name,
            name_reason,
        });
    }
    Ok(rows)
}

fn recipient_key(
    kind: u32,
    reference: u64,
    owner: u32,
    host: u64,
    name: Option<&[u8]>,
    reason: &[u8],
) -> io::Result<String> {
    // Encode every operand rather than hashing names or later display text.
    serde_json::to_string(&(kind, reference, owner, host, name, reason)).map_err(io::Error::other)
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelLabel {
    pub key: String,
    /// Native localiser output with {NUM}/{OIL} operand markers; plain text.
    pub template: Option<String>,
    pub arguments: Vec<String>,
    pub reason: Option<String>,
}

// The fuel-panel breakdown passes NUM text; oil also passes an OIL integer.
// Markers describe the template, not fabricated historical operands or rates.
fn fuel_label(
    localisation: Option<&installed_content_localisation::localisation::Localisation>,
    key: &str,
    oil: bool,
) -> FuelLabel {
    use installed_content_localisation::localisation::Argument;
    let arguments = if oil {
        vec!["NUM".into(), "OIL".into()]
    } else {
        vec!["NUM".into()]
    };
    let (template, mut notes) = match localisation {
        None => (
            None,
            vec!["Recorded fuel-label content unavailable".to_owned()],
        ),
        Some(localisation) if localisation.get(key).is_none() => (
            None,
            vec![format!("Fuel label {key} is absent from recorded content")],
        ),
        Some(localisation) => {
            let operands = [
                ("NUM", Argument::Text("{NUM}")),
                ("OIL", Argument::Text("{OIL}")),
            ];
            let operands = if oil { &operands[..] } else { &operands[..1] };
            let (text, findings) = dashboard_text::catalogue::localised_details(
                localisation,
                key,
                operands,
                dashboard_text::catalogue::View::Plain,
                dashboard_core::findings::Kind::Unsupported,
            );
            (
                text,
                findings
                    .into_iter()
                    .map(|finding| finding.message)
                    .collect(),
            )
        }
    };
    if oil {
        notes.push("Historical OIL count for FUEL_DAILY_GAIN_FROM_OIL is not recorded".into());
    }
    let reason = (!notes.is_empty()).then(|| notes.join("; "));
    FuelLabel {
        key: key.into(),
        template,
        arguments,
        reason,
    }
}

/// Why a read's day is not shown.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum DayLeftOut {
    /// It overlaps a day already shown, so no update is counted twice.
    Overlaps,
    /// The country held no state at some hour of it (or was another country),
    /// so the game did not update its fuel then.
    NotUpdated,
    /// Some hour that governs it was not recorded, or recorded without the
    /// tables that tell whether the country owned a state, so whether the game
    /// updated its fuel is unknown.
    OwnershipNotRecorded,
}

/// One recorded read of the country's fuel status, held over the following hours
/// until the next read. Raw values are exact decimal strings.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelRead {
    pub country: String,
    pub country_instance: Option<u32>,
    /// Capture continuity boundary, independent of the raw surrender target.
    pub continuity_start_hour: Option<i32>,
    /// The hour it was read, where its values start.
    pub first_hour: i32,
    /// The last hour the read's values hold.
    pub last_hour: i32,
    /// The read follows a gap, a missing read or another country identity.
    pub starts_segment: bool,
    /// Fuel in the tank, in [`FuelReply::quantity_scale`] units (`current_raw`).
    pub fuel_raw: String,
    /// The tank's capacity, in the same units (`maximum_raw`).
    pub capacity_raw: String,
    /// Lend-lease fuel received in the hour before the read, in the same units
    /// (`lend_lease_gain_raw`): the tank gains it, but `day` does not include it.
    pub lend_lease_raw: String,
    /// The day of updates before the read; null when `left_out` says why not.
    pub day: Option<FuelDay>,
    pub left_out: Option<DayLeftOut>,
    pub awards_reason: Option<String>,
}

/// A [`FuelRead`] as exact integers; only replies format them.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct RawRead {
    country: u32,
    country_instance: Option<u32>,
    continuity_start_hour: Option<i32>,
    first_hour: i32,
    last_hour: i32,
    starts_segment: bool,
    fuel: i64,
    capacity: i64,
    lend_lease: i64,
    day: Option<RawDay>,
    left_out: Option<DayLeftOut>,
    awards_reason: Option<Arc<str>>,
}

/// A [`FuelDay`]: the ring sums, or the applied updates that supersede them.
#[derive(Clone, Debug, Serialize, Deserialize)]
enum RawDay {
    Ring {
        gained: i64,
        burned: i64,
        received: [i64; 3],
        other: i64,
    },
    Awards(Box<AwardSums>),
}

impl From<&RawRead> for FuelRead {
    fn from(read: &RawRead) -> Self {
        let day = read.day.as_ref().map(|day| match day {
            RawDay::Ring {
                gained,
                burned,
                received,
                other,
            } => FuelDay {
                gained_raw: gained.to_string(),
                burned_raw: burned.to_string(),
                received_raw: received.map(|raw| raw.to_string()),
                other_raw: other.to_string(),
                awards: None,
            },
            RawDay::Awards(sums) => {
                let awards = FuelAwardTotals::from(&**sums);
                FuelDay {
                    gained_raw: awards.own_gain_raw.clone(),
                    burned_raw: awards.consumed_raw.clone(),
                    received_raw: awards.received_raw.clone(),
                    other_raw: awards.transfers_raw[3].clone(),
                    awards: Some(awards),
                }
            }
        });
        Self {
            country: read.country.to_string(),
            country_instance: read.country_instance,
            continuity_start_hour: read.continuity_start_hour,
            first_hour: read.first_hour,
            last_hour: read.last_hour,
            starts_segment: read.starts_segment,
            fuel_raw: read.fuel.to_string(),
            capacity_raw: read.capacity.to_string(),
            lend_lease_raw: read.lend_lease.to_string(),
            day,
            left_out: read.left_out,
            awards_reason: read.awards_reason.as_deref().map(str::to_owned),
        }
    }
}

/// Each distinct refusal text once, shared by every read that repeats it.
#[derive(Default, Serialize, Deserialize)]
struct Reasons(BTreeSet<Arc<str>>);

impl Reasons {
    fn intern(&mut self, reason: &str) -> Arc<str> {
        if let Some(shared) = self.0.get(reason) {
            return Arc::clone(shared);
        }
        let shared = Arc::<str>::from(reason);
        self.0.insert(Arc::clone(&shared));
        shared
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelReply {
    pub branch_id: String,
    pub tag: String,
    /// Units per fuel of the level, capacity and burned values.
    pub quantity_scale: u32,
    /// Units per fuel of the gained values.
    pub gain_scale: u32,
    /// Hours of fuel updates a read's day sums.
    pub day_hours: i32,
    /// Fuel-panel consumer localisation keys. Null names retain their
    /// missing/refused content diagnostic.
    pub consumer_names: [Option<String>; 3],
    pub gain_labels: [FuelLabel; 4],
    /// Incoming LL, outgoing LL, positive flat cost, in that order.
    pub transfer_labels: [FuelLabel; 3],
    pub detail_hours: i32,
    /// In hour order.
    pub reads: Vec<FuelRead>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelAwardsParams {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    pub country: String,
    pub country_instance: u32,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelAwardsReply {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    pub awards: FuelAwards,
    pub warnings: Vec<String>,
}

pub struct CountryFuelAwards;
impl Product for CountryFuelAwards {
    const KEY: &'static str = "country-fuel-awards";
    const CONTENT_DEPENDENT: bool = true;
    type Params = FuelAwardsParams;
    type Reply = FuelAwardsReply;
    fn types() -> Vec<String> {
        vec![
            FuelAwardsParams::decl(),
            FuelAwardPart::decl(),
            FuelAwardTotals::decl(),
            FuelAwards::decl(),
            FuelAwardsReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        CountryParams {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
        }
        .validate()?;
        params
            .country
            .parse::<u32>()
            .map(|_| ())
            .map_err(|_| "Fuel country reference invalid")
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let country = CountryParams {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
        };
        let reads = detail::basis_reads(source, country.branch(), params.hour, cancelled)?;
        let row = detail::authenticated(&reads, params)?;
        let totals =
            FuelAwardTotals::from(&awards_of(row, params.hour).map_err(dashboard_core::invalid)?);
        let catalogue = std::sync::OnceLock::new();
        let recipients = recipients_of(row, cancelled, |bytes| {
            if bytes.is_empty() {
                return Ok(String::new());
            }
            let catalogue = catalogue
                .get_or_init(|| {
                    context
                        .content()
                        .and_then(|options| dashboard_base::catalogue::catalogue(source, options))
                })
                .as_ref()
                .map_err(|error| error.to_string())?;
            // Receipt-time bytes have no literal/generator provenance.
            // DivisionStatus naming at a later read cannot classify them.
            dashboard_text::scripted_text::stored_text(
                &catalogue.localisation,
                &catalogue.recorded_language,
                bytes,
                "fuel recipient cached name",
            )
            .and_then(|text| dashboard_text::catalogue::display_text(text.as_bytes()))
        })?;
        let mut refusals = BTreeMap::new();
        for part in &recipients {
            if part.kind != 1
                && let Some(reason) = &part.name_reason
            {
                *refusals.entry(reason).or_insert(0_usize) += 1;
            }
        }
        let mut warnings: Vec<_> = refusals
            .into_iter()
            .map(|(reason, count)| format!("{count} fuel recipient names refused: {reason}"))
            .collect();
        if let Some(Ok(catalogue)) = catalogue.get() {
            warnings.extend(catalogue.verification.iter().cloned());
            warnings.extend(catalogue.warnings.iter().cloned());
            warnings.extend(catalogue.refusal_lines());
        }
        let reply = FuelAwardsReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            hour: params.hour,
            awards: FuelAwards { totals, recipients },
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply).map_err(|_| {
            dashboard_core::invalid("Fuel award reply exceeds the product byte budget")
        })?;
        Ok(reply)
    }
}

/// A country identity: its canonical slot and native id.
type Identity = (u32, u32);

/// A tag's reads and current consumption as the whole-branch scan gives them.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, Default)]
struct ScannedFuel {
    reads: Vec<RawRead>,
    consumption: Vec<demand::RawConsumption>,
}

#[cfg(any(test, feature = "test-support"))]
impl ScannedFuel {
    /// Its days left out, by [`emit::left_out_code`].
    fn left_out(&self) -> [usize; 3] {
        let mut counts = [0; 3];
        for reason in self.reads.iter().filter_map(|read| read.left_out) {
            counts[emit::left_out_code(reason)] += 1;
        }
        counts
    }
}

/// What a recorded hour tells about the country: its identity (none: the tag was
/// not listed) and whether it owned a state; none when the tables do not tell.
#[cfg(any(test, feature = "test-support"))]
type Holding = Option<(Option<Identity>, bool)>;

#[cfg(any(test, feature = "test-support"))]
fn resolved_slots(
    links: &[replay_data::rows::CountryLinkRow],
) -> std::collections::HashMap<u32, u32> {
    links
        .iter()
        .filter_map(|row| link_slot(links, row.id).map(|slot| (row.id, slot)))
        .collect()
}

#[cfg(any(test, feature = "test-support"))]
fn tagged_identities<'a>(
    countries: impl Iterator<Item = (u32, u32, &'a [u8])>,
    slots: &std::collections::HashMap<u32, u32>,
) -> BTreeMap<&'a [u8], Option<Identity>> {
    let mut tagged = BTreeMap::new();
    for (id, canonical_slot, tag) in countries {
        let identity = tagged.entry(tag).or_insert(None);
        if identity.is_none() && slots.get(&id) == Some(&canonical_slot) {
            *identity = Some((canonical_slot, id));
        }
    }
    tagged
}

#[cfg(any(test, feature = "test-support"))]
fn fuel_by_slot<T>(
    rows: &[T],
    raw: impl Fn(&T) -> u32,
    slot: impl Fn(u32) -> Option<u32>,
) -> BTreeMap<u32, &T> {
    let mut result = BTreeMap::new();
    for row in rows {
        if let Some(slot) = slot(raw(row)) {
            result.entry(slot).or_insert(row);
        }
    }
    result
}

/// What an hour's complete Countries, CountryLinks and States say of the tag's
/// country. Only resolved links are evidence: a tag no country holds is another
/// country (none), and a resolved country owning no state is stateless, but a
/// tag whose country the links cannot resolve, or a country owning no state whose
/// owner some state names by a link that cannot be resolved, cannot tell.
#[cfg(any(test, feature = "test-support"))]
fn holding(identity: Option<Identity>, present: bool, owners: &(BTreeSet<u32>, bool)) -> Holding {
    match identity {
        None if !present => Some((None, false)),
        None => None,
        Some((slot, id)) => {
            if owners.0.contains(&slot) {
                Some((Some((slot, id)), true))
            } else {
                (!owners.1).then_some((Some((slot, id)), false))
            }
        }
    }
}

/// Whether the game updated the fuel of `identity` in each of the 24 updates a
/// read at sample `read` holds: none when it did, else why its day is left out.
/// Those updates are governed by the samples `read` − 24 ..= `read` − 1 (see the
/// module notes), so the read's own sample is not one of them. Each must be
/// recorded (`samples` holds each recorded hour once, ascending) and show the
/// country owning a state. A sample showing no state (or another country)
/// outweighs a missing one, or one that cannot tell.
#[cfg(any(test, feature = "test-support"))]
fn day_left_out(
    samples: &VecDeque<(i32, Holding)>,
    read: i32,
    identity: Identity,
) -> Option<DayLeftOut> {
    let mut unknown = false;
    let mut recorded = 0;
    let governing = samples
        .iter()
        .filter(|(hour, _)| *hour < read && read.saturating_sub(*hour) <= DAY_HOURS);
    for (_, seen) in governing {
        recorded += 1;
        match seen {
            Some((seen, owns)) if *seen != Some(identity) || !owns => {
                return Some(DayLeftOut::NotUpdated);
            }
            Some(_) => {}
            None => unknown = true,
        }
    }
    (unknown || recorded < DAY_HOURS).then_some(DayLeftOut::OwnershipNotRecorded)
}

fn read_of(
    row: &CountryFuelRow,
    hour: i32,
    starts_segment: bool,
    left_out: Option<DayLeftOut>,
    reasons: &mut Reasons,
) -> RawRead {
    let awards = awards_of(row, hour);
    let awards_reason = awards.as_ref().err().map(|reason| reasons.intern(reason));
    // Authenticated completed updates are stronger evidence than the legacy
    // preceding-sample ownership check, which cannot see intervening commands.
    let left_out = if awards.is_ok() && left_out != Some(DayLeftOut::Overlaps) {
        None
    } else {
        left_out
    };
    let day = left_out.is_none().then(|| match awards {
        Ok(awards) => RawDay::Awards(Box::new(awards)),
        Err(_) => RawDay::Ring {
            gained: row.ring_gain_raw,
            burned: row.ring_consumed_raw,
            received: [
                row.ring_received_0_raw,
                row.ring_received_1_raw,
                row.ring_received_2_raw,
            ],
            other: row.ring_other_raw,
        },
    });
    RawRead {
        country: row.country,
        country_instance: match row.country_instance {
            replay_data::rows::Recorded::Value(value) => value,
            _ => None,
        },
        continuity_start_hour: match &row.award_day {
            replay_data::rows::Recorded::Value(day) => day.continuity_start_hour,
            _ => None,
        },
        first_hour: hour,
        last_hour: hour,
        starts_segment,
        fuel: row.current_raw,
        capacity: row.maximum_raw,
        lend_lease: row.lend_lease_gain_raw,
        day,
        left_out,
        awards_reason,
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct CountryScan {
    result: ScannedFuel,
    track: IdentityTrack,
    open: bool,
    demand_open: bool,
    continuity: Option<i32>,
    recent: VecDeque<(i32, Holding)>,
    identity: Option<Identity>,
}

/// Every tag's reads over a branch, and the native countries' tags.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct ScannedBranch {
    countries: BTreeMap<String, Arc<ScannedFuel>>,
    actor_aliases: BTreeMap<String, BTreeSet<String>>,
    unnamed: BTreeSet<u32>,
    actor_conflicts: BTreeSet<String>,
}

#[cfg(any(test, feature = "test-support"))]
impl ScannedBranch {
    fn actor_tags(&self, country: &str) -> impl Iterator<Item = &String> {
        self.actor_aliases.get(country).into_iter().flatten()
    }
}

/// The whole-branch row scan the publication is held to: one all-country
/// observation owns the ownership walk and supplying-read rules.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct Scan {
    countries: BTreeMap<String, CountryScan>,
    active: BTreeSet<String>,
    // Newly observed tags inherit exactly the preceding absent/unknown evidence.
    prior: VecDeque<(i32, Holding)>,
    reasons: Reasons,
    unnamed: BTreeSet<u32>,
}

#[cfg(any(test, feature = "test-support"))]
impl Scan {
    fn observe(
        &mut self,
        decoded: &replay_frame::frame::DecodedFrame,
        format: replay_data::format::FormatVersion,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let hour = decoded.hour;
        let slots = decoded
            .complete::<tables::CountryLinks>()
            .ok()
            .map(resolved_slots);
        let country_keys = decoded
            .complete::<tables::Countries>()
            .ok()
            .map(dashboard_text::metadata::CountryKeys::read);
        let tagged = country_keys
            .as_ref()
            .zip(slots.as_ref())
            .map(|(countries, slots)| tagged_identities(countries.iter(), slots));
        let state_owners: Option<Vec<u32>> = decoded
            .complete::<tables::States>()
            .ok()
            .map(|states| states.iter().map(|state| state.owner).collect());
        let owner_slots = state_owners
            .as_ref()
            .zip(slots.as_ref())
            .map(|(states, links)| {
                let mut owning = BTreeSet::new();
                let mut unresolved = false;
                for owner in states {
                    match links.get(owner).copied() {
                        Some(slot) => {
                            owning.insert(slot);
                        }
                        None => unresolved = true,
                    }
                }
                (owning, unresolved)
            });
        let owners = owner_slots
            .as_ref()
            .filter(|_| country_keys.is_some() && slots.is_some());
        while self
            .prior
            .front()
            .is_some_and(|(seen, _)| hour.saturating_sub(*seen) > DAY_HOURS)
        {
            self.prior.pop_front();
        }
        let mut active = BTreeSet::new();
        if let Some(tagged) = &tagged {
            for (tag, lookup) in tagged {
                match std::str::from_utf8(tag) {
                    Ok(tag) => {
                        let country =
                            self.countries
                                .entry(tag.to_owned())
                                .or_insert_with(|| CountryScan {
                                    recent: self.prior.clone(),
                                    ..Default::default()
                                });
                        country.identity = *lookup;
                        active.insert(tag.to_owned());
                    }
                    Err(_) => {
                        if let Some((_, id)) = lookup {
                            self.unnamed.insert(*id);
                        }
                    }
                }
            }
        }
        let removed: Vec<String> = self.active.difference(&active).cloned().collect();
        for tag in &removed {
            self.countries.get_mut(tag).unwrap().identity = None;
        }
        self.active = active;
        let fuel = decoded.complete::<tables::CountryFuel>().ok();
        let read = slot_read(format, &decoded.metrics, TableId::CountryFuel, hour);
        // A read's packs with their lists, which later formats record apart.
        let assembled = match fuel {
            Some(rows) if read => Some(replay_frame::rows::with_demand_parts(
                rows,
                &decoded.tables[TableId::FuelDemandParts.index()],
                format,
            )?),
            _ => None,
        };
        let by_slot = match (&slots, &assembled) {
            (Some(slots), Some(rows)) if country_keys.is_some() => {
                fuel_by_slot(rows, |row| row.country, |raw| slots.get(&raw).copied())
            }
            _ => BTreeMap::new(),
        };
        for tag in self.active.iter().chain(&removed) {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let country = self.countries.get_mut(tag).unwrap();
            let identity = country.identity;
            let seen =
                owners.and_then(|owners| holding(identity, self.active.contains(tag), owners));
            // Dormant tags share the preceding absent/unknown proof. Restore only
            // that bounded suffix when the tag reappears, never its old history.
            let last = country.recent.back().map(|(hour, _)| *hour);
            if last.and_then(|last| last.checked_add(1)) != Some(hour) {
                country.recent.extend(
                    self.prior
                        .iter()
                        .filter(|(hour, _)| last.is_none_or(|last| *hour > last))
                        .copied(),
                );
            }
            while country
                .recent
                .front()
                .is_some_and(|(seen, _)| hour.saturating_sub(*seen) > DAY_HOURS)
            {
                country.recent.pop_front();
            }
            country.recent.push_back((hour, seen));
            let supplied = identity
                .filter(|_| fuel.is_some() && read)
                .and_then(|identity| by_slot.get(&identity.0))
                .map(|row| {
                    (
                        read_of(row, hour, false, None, &mut self.reasons),
                        demand::summary_of(row, hour, hour, false, &mut self.reasons),
                    )
                });
            accept_country(country, hour, fuel.is_some(), read, supplied);
        }
        self.prior.push_back((hour, owners.map(|_| (None, false))));
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
fn accept_country(
    country: &mut CountryScan,
    hour: i32,
    available: bool,
    read: bool,
    supplied: Option<(RawRead, demand::RawConsumption)>,
) {
    let continuous = country.track.step(country.identity, hour);
    country.open &= continuous;
    country.demand_open &= continuous;
    let Some(identity) = country.identity.filter(|_| available) else {
        country.open = false;
        country.demand_open = false;
        return;
    };
    if !read {
        if country.open
            && let Some(read) = country.result.reads.last_mut()
        {
            read.last_hour = hour;
        }
        if country.demand_open
            && let Some(read) = country.result.consumption.last_mut()
        {
            read.last_hour = hour;
        }
        return;
    }
    let Some((mut fuel, mut consumption)) = supplied else {
        country.open = false;
        country.demand_open = false;
        return;
    };
    if country.continuity != fuel.continuity_start_hour {
        country.open = false;
    }
    country.continuity = fuel.continuity_start_hour;
    fuel.starts_segment = !country.open;
    if fuel.awards_reason.is_some() {
        fuel.left_out = day_left_out(&country.recent, hour, identity);
        if fuel.left_out.is_some() {
            fuel.day = None;
        }
    }
    consumption.segment_start_hour = if country.demand_open {
        country
            .result
            .consumption
            .last()
            .map_or(hour, |read| read.segment_start_hour)
    } else {
        hour
    };
    consumption.starts_segment = !country.demand_open;
    country.result.reads.push(fuel);
    country.result.consumption.push(consumption);
    country.open = true;
    country.demand_open = true;
}

/// Leaves out the day of every read that overlaps a day kept. Reads in the fuel
/// table's daily slot come first, then the others, each in hour order; a day is
/// kept when its read is at least [`DAY_HOURS`] from every read kept before it,
/// which every slot read is. Days already left out never hold another back.
#[cfg(any(test, feature = "test-support"))]
fn keep_days(format: replay_data::format::FormatVersion, reads: &mut [RawRead]) {
    let slot = cadence::cadence_in(TableId::CountryFuel, format);
    let in_slot = |read: &RawRead| slot.is_refresh(read.first_hour);
    let mut kept = BTreeSet::new();
    let (slot, other): (Vec<_>, Vec<_>) = reads
        .iter_mut()
        .filter(|read| read.day.is_some())
        .partition(|read| in_slot(read));
    for read in slot.into_iter().chain(other) {
        let near = read.first_hour.saturating_sub(DAY_HOURS - 1)
            ..=read.first_hour.saturating_add(DAY_HOURS - 1);
        if kept.range(near).next().is_some() {
            read.day = None;
            read.left_out = Some(DayLeftOut::Overlaps);
        } else {
            kept.insert(read.first_hour);
        }
    }
}

impl Product for CountryFuel {
    const KEY: &'static str = "country-fuel";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = CountryParams;
    type Reply = FuelReply;

    fn types() -> Vec<String> {
        vec![
            FuelAwardPart::decl(),
            FuelAwardTotals::decl(),
            FuelAwards::decl(),
            FuelLabel::decl(),
            FuelDay::decl(),
            DayLeftOut::decl(),
            FuelRead::decl(),
            FuelReply::decl(),
            query::FuelSeriesPoint::decl(),
            query::FuelSeriesStateReply::decl(),
            query::FuelSeriesLine::decl(),
            query::FuelSeriesSummaryReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = scanned(source, params.branch(), cancelled)?;
        let empty = ScannedFuel::default();
        let scanned = branch
            .countries
            .get(&params.tag)
            .map(Arc::as_ref)
            .unwrap_or(&empty);
        let counted = Counted {
            read: !scanned.reads.is_empty(),
            left_out: scanned.left_out(),
        };
        let mut reply = reply(source, params, context, &branch.unnamed, counted)?;
        reply.reads = scanned.reads.iter().map(FuelRead::from).collect();
        Ok(reply)
    }
}

/// What a reply's warnings count of a tag's reads: whether it had any, and its days left out
/// by [`emit::left_out_code`].
#[derive(Clone, Copy)]
struct Counted {
    read: bool,
    left_out: [usize; 3],
}

/// A tag's reply with its warnings, its reads empty.
fn reply(
    source: &Source,
    params: &CountryParams,
    context: &LoadContext,
    unnamed: &BTreeSet<u32>,
    counted: Counted,
) -> io::Result<FuelReply> {
    let mut warnings = Vec::new();
    if !unnamed.is_empty() {
        warnings.push(format!(
            "{} fuel countries have non-UTF-8 tags and cannot be selected by tag: {:?}",
            unnamed.len(),
            unnamed
        ));
    }
    if !counted.read {
        warnings.push(format!("{}: no recorded fuel in this branch", params.tag));
    }
    let left_out = |reason| counted.left_out[emit::left_out_code(reason)];
    let overlapping = left_out(DayLeftOut::Overlaps);
    if overlapping > 0 {
        warnings.push(format!(
                "Fuel reads less than a day from another: {overlapping}. Their gain and burn overlap a day already shown and are left out."
            ));
    }
    let not_updated = left_out(DayLeftOut::NotUpdated);
    if not_updated > 0 {
        warnings.push(format!(
                "Fuel reads after a day the country held no state: {not_updated}. The game did not update its fuel then, so their gain and burn are left out."
            ));
    }
    let unknown = left_out(DayLeftOut::OwnershipNotRecorded);
    if unknown > 0 {
        warnings.push(format!(
                "Fuel reads after a day recorded without state ownership: {unknown}. Whether the game updated the fuel then is unknown, so their gain and burn are left out."
            ));
    }
    let catalogue = match context
        .content()
        .and_then(|options| dashboard_base::catalogue::catalogue(source, options))
    {
        Ok(catalogue) => {
            warnings.extend(catalogue.verification.iter().cloned());
            warnings.extend(catalogue.warnings.iter().cloned());
            Some(catalogue)
        }
        Err(why) => {
            warnings.push(format!("10 fuel labels cannot be resolved: {why}"));
            None
        }
    };
    let consumer_names = consumer_names(catalogue.as_deref(), &mut warnings);
    let localisation = catalogue
        .as_ref()
        .map(|catalogue| catalogue.localisation.as_ref());
    // Category templates retain their NUM/OIL operands.
    let gain_labels = [
        "FUEL_DAILY_GAIN_BASE",
        "FUEL_DAILY_GAIN_FROM_STATES",
        "FUEL_DAILY_GAIN_FROM_OIL",
        "FUEL_DAILY_GAIN_FROM_OTHER",
    ]
    .map(|key| fuel_label(localisation, key, key == "FUEL_DAILY_GAIN_FROM_OIL"));
    let transfer_labels = [
        "FUEL_DAILY_GAIN_FROM_LEND_LEASE",
        "FUEL_DAILY_REQUIRED_FROM_LEND_LEASE",
        "FUEL_DAILY_REQUIRED_FROM_OTHER",
    ]
    .map(|key| fuel_label(localisation, key, false));
    for label in gain_labels.iter().chain(&transfer_labels) {
        if let Some(reason) = &label.reason {
            warnings.push(format!("1 fuel label {}: {reason}", label.key));
        }
    }
    if let Some(catalogue) = catalogue {
        warnings.extend(catalogue.refusal_lines());
    }
    Ok(FuelReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        quantity_scale: QUANTITY_UNITS,
        gain_scale: GAIN_UNITS,
        day_hours: DAY_HOURS,
        consumer_names,
        gain_labels,
        transfer_labels,
        detail_hours: detail::DETAIL_HOURS,
        reads: Vec::new(),
        warnings,
    })
}

/// The fuel panel's labels of the country's three fuel consumers, in their
/// native order, through the recorded content. A label the content lacks or
/// refuses is None, with a warning naming its key.
pub fn consumer_names(
    catalogue: Option<&dashboard_base::catalogue::Catalogue>,
    warnings: &mut Vec<String>,
) -> [Option<String>; 3] {
    ["TOPBAR_ARMY", "TOPBAR_AIR", "TOPBAR_NAVY"].map(|key| {
        let name = catalogue.and_then(|catalogue| {
            catalogue
                .localisation
                .get(key)
                .is_some()
                .then(|| catalogue.localised(key, dashboard_text::catalogue::View::Plain))
                .flatten()
        });
        if name.is_none() {
            warnings.push(format!(
                "Fuel consumer label {key} is absent or refused by the recorded content"
            ));
        }
        name
    })
}

#[cfg(test)]
mod tests;
