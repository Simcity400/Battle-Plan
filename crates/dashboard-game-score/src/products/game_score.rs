//! End-screen country scores at each acquisition read, never cumulative totals.
//! The end screen calls current getters rather than hourly Army/Navy/Air caches.
//! Industry and Land are separate from Total.

use dashboard_base::{
    products::BranchParams,
    products::LoadContext,
    products::MAX_PRODUCT_REPLY_BYTES,
    products::Product,
    products::country_history::{SlotRows, unique_by_slot},
    products::fixed5_runs::{Fixed5State, NativeFixed5Run, push_fixed5},
    products::status_missing_reason,
};

use dashboard_base::{Source, catalogue, content, prepared::frame_changes::field_mask};
use dashboard_text::{
    faction_names,
    metadata::{self, TaggedCountry, tagged_countries},
};
use dashboard_wars::products::war_casualties::same_name_rows;
use dashboard_world::{Mark, Viewed};
use installed_content_core::faction_templates::Catalog;
use installed_content_country_names::{country_names, country_names::CountryNames};
use installed_content_localisation::localisation::Localisation;
use replay::campaign::Replay;
use replay_data::{
    BranchId, format::FormatVersion, rows::AirBaseRegistrationRow,
    rows::AirWingGameScoreStatsRow, rows::CountryAppearanceRow, rows::CountryFactoriesRow,
    rows::CountryGameScoreInputRow, rows::CountryLinkRow, rows::CountryRow,
    rows::FactionMemberRow, rows::GameScoreAirSite, rows::GameScoreMultiplierRow, rows::RowCodec,
    rows::TableId, rows::TableSchema, rows::mask, rows::tables,
};
use replay_frame::{
    frame::DecodedFrame, frame::HourFrame, frame::OperandStatus,
    frame::Rowset, frame::SharedView, frame::TableView,
};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{
        LazyLock,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct GameScore;
pub mod emit;
pub mod facts;
pub mod query;
#[cfg(any(test, feature = "test-support"))]
pub mod row;
pub mod served;
const Q: i64 = 100_000;
const PARTS: [&str; 6] = ["army", "navy", "air", "industry", "land", "total"];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
pub enum GameScoreMissingKind {
    Recording,
    Content,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
pub struct GameScoreMissing {
    pub kind: GameScoreMissingKind,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct GameScoreAmount {
    /// Signed native fixed5.
    pub raw: Option<i64>,
    pub missing: Option<GameScoreMissing>,
    /// Runs cover consecutive fresh hourly observations. When true, the supplying
    /// read is the queried hour inside the run, including equal-value rereads.
    /// Content-evaluated multipliers have no recorded read date.
    pub read_each_hour: bool,
}

impl GameScoreAmount {
    fn known(raw: i64, read_each_hour: bool) -> Self {
        Self {
            raw: Some(raw),
            missing: None,
            read_each_hour,
        }
    }
    fn recording(reason: impl Into<String>, read_each_hour: bool) -> Self {
        Self {
            raw: None,
            missing: Some(GameScoreMissing {
                kind: GameScoreMissingKind::Recording,
                reason: reason.into(),
            }),
            read_each_hour,
        }
    }
    fn content(reason: impl Into<String>) -> Self {
        Self {
            raw: None,
            missing: Some(GameScoreMissing {
                kind: GameScoreMissingKind::Content,
                reason: reason.into(),
            }),
            read_each_hour: false,
        }
    }
    fn value(&self) -> Option<i64> {
        self.raw
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct GameScoreValues {
    pub army: GameScoreAmount,
    pub navy: GameScoreAmount,
    pub air: GameScoreAmount,
    pub industry: GameScoreAmount,
    pub land: GameScoreAmount,
    pub total: GameScoreAmount,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct GameScoreSeries {
    pub army: Vec<NativeFixed5Run>,
    pub navy: Vec<NativeFixed5Run>,
    pub air: Vec<NativeFixed5Run>,
    pub industry: Vec<NativeFixed5Run>,
    pub land: Vec<NativeFixed5Run>,
    pub total: Vec<NativeFixed5Run>,
    #[serde(skip)]
    #[ts(skip)]
    states: [Fixed5State; 6],
}

#[derive(Default)]
struct ReasonTable {
    entries: Vec<GameScoreMissing>,
    indices: BTreeMap<GameScoreMissing, usize>,
}

impl ReasonTable {
    fn index(&mut self, missing: Option<&GameScoreMissing>) -> Option<usize> {
        let missing = missing?;
        if let Some(index) = self.indices.get(missing) {
            return Some(*index);
        }
        let index = self.entries.len();
        self.entries.push(missing.clone());
        self.indices.insert(missing.clone(), index);
        Some(index)
    }
}

impl GameScoreSeries {
    fn push(
        &mut self,
        hour: i32,
        values: &GameScoreValues,
        break_before: bool,
        reasons: &mut ReasonTable,
        budget: &mut usize,
    ) -> io::Result<()> {
        let runs = [
            &mut self.army,
            &mut self.navy,
            &mut self.air,
            &mut self.industry,
            &mut self.land,
            &mut self.total,
        ];
        for ((runs, last), amount) in runs.into_iter().zip(&mut self.states).zip(values.amounts()) {
            let missing = reasons.index(amount.missing.as_ref());
            let added = push_fixed5(
                runs,
                last,
                hour,
                amount.value(),
                missing,
                amount.read_each_hour,
                break_before,
            );
            *budget = budget.checked_add(added).ok_or_else(oversized)?;
            if *budget > MAX_PRODUCT_REPLY_BYTES {
                return Err(oversized());
            }
        }
        Ok(())
    }
}

impl GameScoreValues {
    fn amounts(&self) -> [&GameScoreAmount; 6] {
        [
            &self.army,
            &self.navy,
            &self.air,
            &self.industry,
            &self.land,
            &self.total,
        ]
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct GameScoreRun<T> {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: T,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
pub struct GameScoreFaction {
    pub id: String,
    pub name: Option<String>,
    pub name_missing: Option<GameScoreMissing>,
    pub ordinal: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
pub struct GameScoreIdentity {
    pub canonical_slot: Option<u32>,
    pub native_country: Option<u32>,
    pub name: Option<String>,
    pub name_missing: Option<GameScoreMissing>,
    pub color: Option<[u8; 3]>,
    pub color_missing: Option<GameScoreMissing>,
    /// None with no missing reason is known no faction; unknown is explicit.
    pub faction: Option<GameScoreFaction>,
    pub faction_missing: Option<GameScoreMissing>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct GameScoreCountry {
    pub tag: String,
    pub identity: Vec<GameScoreRun<GameScoreIdentity>>,
    pub scores: GameScoreSeries,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct GameScoreWarning {
    pub kind: GameScoreMissingKind,
    pub reason: String,
    #[ts(type = "number")]
    pub observations: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GameScoreReply {
    pub branch_id: String,
    #[ts(type = "number")]
    pub scale: i64,
    /// Every observed acquisition hour, even when all values equal the prior read.
    /// Use dashboard/reads.ts with run bounds; no read crosses a recording gap.
    pub reads: Vec<i32>,
    pub countries: Vec<GameScoreCountry>,
    /// The same six native multipliers, globally, not per-country difficulty.
    pub multipliers: GameScoreSeries,
    pub reasons: Vec<GameScoreMissing>,
    /// Plain UI column labels, as chosen by the user.
    pub labels: BTreeMap<String, String>,
    /// Localised native sort-button tooltips, absent when content cannot supply them.
    pub hints: BTreeMap<String, String>,
    pub warnings: Vec<String>,
    pub score_warnings: Vec<GameScoreWarning>,
}

// Preserve both low-64 products before signed /100000.
// Cancelling the final scale changes overflow.
fn count_score(count: i32, multiplier: i64) -> i64 {
    i64::from(count).wrapping_mul(multiplier).wrapping_mul(Q) / Q
}

// Truncate each total, narrow to i32, then add/subtract in this order.
fn industry_count(row: &CountryFactoriesRow) -> i32 {
    ((row.naval_total_raw / Q) as i32)
        .wrapping_add((row.civilian_total_raw / Q) as i32)
        .wrapping_add((row.military_total_raw / Q) as i32)
        .wrapping_sub(row.civilian_to_others)
        .wrapping_sub(row.civilian_to_overlord)
        .wrapping_sub(row.military_to_others)
        .wrapping_sub(row.military_to_overlord)
}

// Every air occurrence contributes independently; the selected stat
// is a stored wing cache.
fn air_contribution(planes: i32, stat: i64) -> i64 {
    stat.wrapping_mul(i64::from(planes).wrapping_mul(Q)) / Q
}

// Total excludes Industry and Land.
fn total_score(army: i64, navy: i64, air: i64, multiplier: i64) -> i64 {
    air.wrapping_add(navy)
        .wrapping_add(army)
        .wrapping_mul(multiplier)
        / Q
}

fn recording_missing(reason: impl Into<String>) -> GameScoreMissing {
    GameScoreMissing {
        kind: GameScoreMissingKind::Recording,
        reason: reason.into(),
    }
}

fn bytes_reason(bytes: &[u8], fallback: &str) -> String {
    if bytes.is_empty() {
        fallback.to_owned()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn captured(value: Option<i64>, error: &[u8], what: &str) -> GameScoreAmount {
    match value {
        Some(value) => GameScoreAmount::known(value, true),
        None => GameScoreAmount::recording(bytes_reason(error, what), true),
    }
}

fn multiply_count(
    count: Option<i32>,
    error: &[u8],
    what: &str,
    multiplier: &GameScoreAmount,
) -> GameScoreAmount {
    match (count, multiplier.value()) {
        (Some(count), Some(multiplier)) => {
            GameScoreAmount::known(count_score(count, multiplier), true)
        }
        (None, _) => GameScoreAmount::recording(bytes_reason(error, what), true),
        (_, None) => multiplier.clone(),
    }
}

fn multipliers(row: Option<&GameScoreMultiplierRow>, absent: &str) -> GameScoreValues {
    let Some(row) = row else {
        return unknown_values(absent);
    };
    GameScoreValues {
        total: captured(
            row.total_raw,
            &row.total_error,
            "COUNTRY_SCORE_MULTIPLIER not read",
        ),
        army: captured(
            row.army_raw,
            &row.army_error,
            "ARMY_SCORE_MULTIPLIER not read",
        ),
        navy: captured(
            row.navy_raw,
            &row.navy_error,
            "NAVY_SCORE_MULTIPLIER not read",
        ),
        air: captured(row.air_raw, &row.air_error, "AIR_SCORE_MULTIPLIER not read"),
        industry: captured(
            row.industry_raw,
            &row.industry_error,
            "INDUSTRY_SCORE_MULTIPLIER not read",
        ),
        land: captured(
            row.land_raw,
            &row.land_error,
            "PROVINCE_SCORE_MULTIPLIER not read",
        ),
    }
}

fn unknown_values(reason: &str) -> GameScoreValues {
    let missing = || GameScoreAmount::recording(reason, false);
    GameScoreValues {
        army: missing(),
        navy: missing(),
        air: missing(),
        industry: missing(),
        land: missing(),
        total: missing(),
    }
}

fn multipliers_at(
    frame: HourFrame<'_>,
    old: bool,
    old_multipliers: &mut impl FnMut() -> GameScoreValues,
) -> io::Result<GameScoreValues> {
    if old
        || matches!(frame.status(TableId::GameScoreMultipliers),
        OperandStatus::Unavailable(reason) if reason == replay_data::format::GAME_SCORE_DISABLED_REASON)
    {
        return Ok(old_multipliers());
    }
    let observed = frame.complete::<tables::GameScoreMultipliers>(*MULTIPLIER_FIELDS);
    let absent = missing_reason(frame, TableId::GameScoreMultipliers).unwrap_or_else(|| {
        "GameScoreMultipliers does not contain one singleton at this read".into()
    });
    let row = match observed.ok().filter(|rows| rows.len() == 1) {
        Some(rows) => rows.iter().next().transpose()?,
        None => None,
    };
    Ok(multipliers(row.as_deref(), &absent))
}

/// Why `table` holds no complete rows this hour, as [`status_missing_reason`] says it; none
/// where it is complete.
fn missing_reason(frame: HourFrame<'_>, table: TableId) -> Option<String> {
    status_missing_reason(frame.status(table), table)
}

/// The rows `T` holds this hour, complete or truncated, each holding the fields `mask` names; an
/// unavailable table holds none, as [`metadata::recorded_rows`] reads them.
fn recorded<T: TableSchema>(frame: HourFrame<'_>, mask: u64) -> io::Result<Rowset<'_, T::Row>> {
    match frame.status(T::ID) {
        OperandStatus::NotRecorded(mode) => Err(dashboard_core::invalid(metadata::not_recorded(
            frame.hour(),
            T::ID,
            mode,
        ))),
        OperandStatus::WrongLayout | OperandStatus::TruncatedWrongLayout(_) => {
            Err(dashboard_core::invalid(format!(
                "hour {} {:?} contains another table's rows",
                frame.hour(),
                T::ID
            )))
        }
        _ => Ok(frame.held::<T>(mask)),
    }
}

/// `rows` as one slice in key order: a decoded frame's borrowed, a walk's built in the fields
/// its mask names.
fn slice<'a, R: RowCodec + Clone>(rows: Rowset<'a, R>) -> io::Result<Cow<'a, [R]>> {
    match rows {
        Rowset::Decoded(rows) => Ok(Cow::Borrowed(rows)),
        walked => walked
            .iter()
            .map(|row| row.map(Cow::into_owned))
            .collect::<io::Result<Vec<R>>>()
            .map(Cow::Owned),
    }
}

/// [`slice`] of the complete `T`; none where it is not complete.
fn complete<T: TableSchema>(
    frame: HourFrame<'_>,
    mask: u64,
) -> io::Result<Option<Cow<'_, [T::Row]>>>
where
    T::Row: Clone,
{
    frame.complete::<T>(mask).ok().map(slice).transpose()
}

// Defines::fixed5 owns Lua double -> f32 and separate multiply, add, subtract
// and truncating integer conversion. Old recordings have no runtime define read.
fn content_multipliers(
    defines: Result<&installed_content_core::defines::Defines, &str>,
) -> GameScoreValues {
    let read = |key: &str| match defines {
        Ok(defines) => match defines.fixed5("NCountry", key) {
            Ok(raw) => GameScoreAmount::known(raw, false),
            Err(error) => GameScoreAmount::content(error.to_string()),
        },
        Err(reason) => GameScoreAmount::content(format!("NDefines.NCountry.{key}: {reason}")),
    };
    GameScoreValues {
        total: read("COUNTRY_SCORE_MULTIPLIER"),
        army: read("ARMY_SCORE_MULTIPLIER"),
        navy: read("NAVY_SCORE_MULTIPLIER"),
        air: read("AIR_SCORE_MULTIPLIER"),
        industry: read("INDUSTRY_SCORE_MULTIPLIER"),
        land: read("PROVINCE_SCORE_MULTIPLIER"),
    }
}

fn push_run<T: PartialEq + Serialize>(
    runs: &mut Vec<GameScoreRun<T>>,
    hour: i32,
    value: T,
    break_before: bool,
    budget: &mut usize,
) -> io::Result<()> {
    if let Some(last) = runs.last_mut()
        && !break_before
        && last.last_hour.checked_add(1) == Some(hour)
        && last.value == value
    {
        last.last_hour = hour;
        return Ok(());
    }
    let run = GameScoreRun {
        first_hour: hour,
        last_hour: hour,
        value,
    };
    // Sum of run payloads is a lower bound on the full reply. Refuse only when
    // even that lower bound exceeds the transport budget; final check is exact.
    let length = dashboard_core::transport::json_len(&run.value, MAX_PRODUCT_REPLY_BYTES)
        .ok_or_else(oversized)?;
    *budget = budget.checked_add(length).ok_or_else(oversized)?;
    if *budget > MAX_PRODUCT_REPLY_BYTES {
        return Err(oversized());
    }
    runs.push(run);
    Ok(())
}

fn oversized() -> io::Error {
    dashboard_core::invalid("game_score reply exceeds MAX_PRODUCT_REPLY_BYTES")
}

fn air_score(
    frame: HourFrame<'_>,
    row: &CountryGameScoreInputRow,
    slot: u32,
    multiplier: &GameScoreAmount,
    // Complete tables, read by their unique keys.
    wings: Option<Rowset<'_, AirWingGameScoreStatsRow>>,
    registrations: Option<Rowset<'_, AirBaseRegistrationRow>>,
) -> io::Result<GameScoreAmount> {
    match row.air_state {
        CountryGameScoreInputRow::AIR_ABSENT => return Ok(GameScoreAmount::known(0, true)),
        CountryGameScoreInputRow::AIR_PRESENT => {}
        _ => {
            return Ok(GameScoreAmount::recording(
                bytes_reason(&row.air_error, "strategic-air entry unreadable"),
                true,
            ));
        }
    }
    let sum = (|| -> io::Result<Result<i64, String>> {
        let mut sum = 0_i64;
        for site in &row.first_sites {
            match site.access {
                GameScoreAirSite::ACCESS_FALSE => continue, // Access is false regardless of capacity.
                GameScoreAirSite::ACCESS_TRUE => {}
                _ => return Ok(Err(bytes_reason(&site.error, "air-base access unreadable"))),
            }
            let Some(registrations) = registrations else {
                return Ok(Err(missing_reason(frame, TableId::AirBaseRegistrations)
                    .unwrap_or_else(|| "AirBaseRegistrations join unavailable".into())));
            };
            let Some(registration) =
                registrations.get(u128::from(site.base) << 32 | u128::from(slot))?
            else {
                return Ok(Err(format!(
                    "accessible air base {} has no AirBaseRegistrations row for country slot {slot}",
                    site.base
                )));
            };
            for id in &registration.wings {
                let Some(wings) = wings else {
                    return Ok(Err(missing_reason(frame, TableId::AirWingGameScoreStats)
                        .unwrap_or_else(|| "AirWingGameScoreStats join unavailable".into())));
                };
                let Some(wing) = wings.get(u128::from(*id))? else {
                    return Ok(Err(format!(
                        "air wing {id} has no recorded mission-1 score-stat companion"
                    )));
                };
                let Some(planes) = wing.planes else {
                    return Ok(Err(bytes_reason(
                        &wing.planes_error,
                        "air-wing plane count unreadable",
                    )));
                };
                let Some(stat) = wing.stat_raw else {
                    return Ok(Err(bytes_reason(
                        &wing.stat_error,
                        "air-wing mission-1 stat 0x33 unreadable",
                    )));
                };
                sum = sum.wrapping_add(air_contribution(planes, stat));
            }
        }
        Ok(Ok(sum))
    })()?;
    Ok(match (sum, multiplier.value()) {
        (Ok(sum), Some(multiplier)) => {
            GameScoreAmount::known(sum.wrapping_mul(multiplier) / Q, true)
        }
        (Err(reason), _) => GameScoreAmount::recording(reason, true),
        (_, None) => multiplier.clone(),
    })
}

struct ScoreContext<'a> {
    frame: HourFrame<'a>,
    multipliers: &'a GameScoreValues,
    input_refusal: Option<&'a str>,
    wings: Option<Rowset<'a, AirWingGameScoreStatsRow>>,
    registrations: Option<Rowset<'a, AirBaseRegistrationRow>>,
}

/// One stateless observation retains diagnostics for the existing reply builder.
struct ScoreSamples {
    multipliers: GameScoreValues,
    countries: BTreeMap<String, (GameScoreValues, Option<(u32, u32)>)>,
    warnings: BTreeMap<(GameScoreMissingKind, String), u64>,
}

fn sample(
    frame: HourFrame<'_>,
    format: replay_data::format::FormatVersion,
    fallback: &mut impl FnMut() -> GameScoreValues,
) -> io::Result<ScoreSamples> {
    let old = !TableId::CountryGameScoreInputs
        .last_change()
        .present(format.version());
    let input_refusal = format.table_refusal(TableId::CountryGameScoreInputs);
    let multipliers = multipliers_at(frame, old, fallback)?;
    let mut warnings = BTreeMap::new();
    for amount in multipliers.amounts() {
        note_missing(amount.missing.as_ref(), &mut warnings);
    }
    let mut countries = BTreeMap::new();
    if let (Some(country_rows), Some(links)) = (
        complete::<tables::Countries>(frame, *HOLDER_FIELDS)?,
        complete::<tables::CountryLinks>(frame, LINK_FIELDS)?,
    ) {
        let (country_rows, links) = (&*country_rows, &*links);
        let input_rows = complete::<tables::CountryGameScoreInputs>(frame, *INPUT_FIELDS)?;
        let input_rows = input_rows.as_deref().unwrap_or(&[]);
        let inputs = unique_by_slot(input_rows, links, |row| row.country);
        for row in input_rows {
            if metadata::link_slot(links, row.country).is_none() {
                *warnings.entry(orphan_input(row.country)).or_default() += 1;
            }
        }
        let factory_rows = complete::<tables::CountryFactories>(frame, *FACTORY_FIELDS)?;
        let factories = unique_by_slot(
            factory_rows.as_deref().unwrap_or(&[]),
            links,
            |row| row.country,
        );
        let context = ScoreContext {
            frame,
            multipliers: &multipliers,
            input_refusal: input_refusal.as_deref(),
            wings: frame
                .complete::<tables::AirWingGameScoreStats>(*WING_FIELDS)
                .ok(),
            registrations: frame
                .complete::<tables::AirBaseRegistrations>(*REGISTRATION_FIELDS)
                .ok(),
        };
        for (tag, lookup) in tagged_countries(country_rows, links) {
            let Ok(tag) = std::str::from_utf8(tag) else {
                *warnings
                    .entry((
                        GameScoreMissingKind::Recording,
                        format!("game_score country tag bytes {tag:02x?} refused: not UTF-8"),
                    ))
                    .or_default() += 1;
                continue;
            };
            let (values, holder) = match lookup {
                TaggedCountry::Found(row, slot) => (
                    context.scores(
                        inputs.rows.get(&slot).copied().flatten(),
                        slot,
                        factories.rows.get(&slot).copied().flatten(),
                    )?,
                    Some((slot, row.id)),
                ),
                _ => (
                    unknown_values("Country identity is unresolved by CountryLinks"),
                    None,
                ),
            };
            for amount in values.amounts() {
                note_missing(amount.missing.as_ref(), &mut warnings);
            }
            countries.insert(tag.to_owned(), (values, holder));
        }
    } else {
        *warnings
            .entry((
                GameScoreMissingKind::Recording,
                "Countries or CountryLinks is incomplete; country identities are unknown".into(),
            ))
            .or_default() += 1;
    }
    Ok(ScoreSamples {
        multipliers,
        countries,
        warnings,
    })
}

impl ScoreContext<'_> {
    fn scores(
        &self,
        input: Option<&CountryGameScoreInputRow>,
        slot: u32,
        factories: Option<&CountryFactoriesRow>,
    ) -> io::Result<GameScoreValues> {
        let Self {
            frame,
            multipliers,
            input_refusal,
            wings,
            registrations,
        } = *self;
        let input_missing = missing_reason(frame, TableId::CountryGameScoreInputs);
        let mut result = unknown_values(input_refusal.or(input_missing.as_deref()).unwrap_or(
            "CountryGameScoreInputs is complete but this country's row is missing or ambiguous",
        ));
        if input_refusal.is_none()
            && let Some(row) = input
        {
            result.army = multiply_count(
                row.army_count,
                &row.army_error,
                "division roster header unreadable",
                &multipliers.army,
            );
            result.navy = multiply_count(
                row.navy_count,
                &row.navy_error,
                "fleet task-force count sum unreadable",
                &multipliers.navy,
            );
            result.land = multiply_count(
                row.land_count,
                &row.land_error,
                "controlled-province header unreadable",
                &multipliers.land,
            );
            result.air = air_score(frame, row, slot, &multipliers.air, wings, registrations)?;
            result.total = match (
                result.army.value(),
                result.navy.value(),
                result.air.value(),
                multipliers.total.value(),
            ) {
                (Some(army), Some(navy), Some(air), Some(multiplier)) => {
                    GameScoreAmount::known(total_score(army, navy, air, multiplier), true)
                }
                _ => GameScoreAmount::recording(
                    "Total needs Army, Navy, Air and COUNTRY_SCORE_MULTIPLIER at one coherent read",
                    true,
                ),
            };
        }
        result.industry = match (factories, multipliers.industry.value()) {
            (Some(row), Some(multiplier)) => GameScoreAmount::known(count_score(industry_count(row), multiplier), true),
            (None, _) => GameScoreAmount::recording(missing_reason(frame, TableId::CountryFactories)
                .unwrap_or_else(|| "CountryFactories is complete but this country's row is missing or ambiguous".into()), false),
            (_, None) => multipliers.industry.clone(),
        };
        Ok(result)
    }
}

type FactionName = (Option<String>, Option<GameScoreMissing>);

/// The lexical dictionary faction names read, held while LexicalTokens is the one it read.
struct FactionLexical {
    tokens: faction_names::LexicalIndex,
    missing: Option<String>,
    read: Mark,
}

impl FactionLexical {
    fn new(hour: ScoreHour<'_>) -> Self {
        Self {
            tokens: faction_names::LexicalIndex::default(),
            missing: missing_reason(hour.frame, TableId::LexicalTokens),
            read: hour.mark(TableId::LexicalTokens),
        }
    }

    /// Whether `hour` holds the LexicalTokens it read.
    fn holds(&self, hour: ScoreHour<'_>) -> bool {
        hour.is(TableId::LexicalTokens, &self.read)
    }
}

fn faction_names_at(
    frame: HourFrame<'_>,
    localisation: Option<&Localisation>,
    fallback: &Localisation,
    recorded_language: &str,
    templates: Result<&Catalog, String>,
    lexical: &FactionLexical,
) -> io::Result<Option<HashMap<u64, FactionName>>> {
    let Ok(rows) = frame.complete::<tables::Factions>(*FACTION_NAME_FIELDS) else {
        return Ok(None);
    };
    let rows = rows.iter().collect::<io::Result<Vec<_>>>()?;
    Ok(Some(
        rows.iter()
            .map(|row| {
                // Faction-name literals bypass both the template and lexical dictionary;
                // empty literals use the shared bound-name rule.
                let template = match (row.template_token, &lexical.missing) {
                    (Some(_), Some(reason)) => Err(reason.clone()),
                    _ => faction_names::template_spelling_in(&frame, row.template_token),
                };
                let templates = if localisation.is_none() {
                    Err("Faction naming localisation is unavailable".to_owned())
                } else {
                    templates.as_ref().copied().map_err(Clone::clone)
                };
                let mut recording_failure = false;
                let resolved = faction_names::name(
                    &row.name,
                    template
                        .as_ref()
                        .map(|value| value.as_deref())
                        .map_err(Clone::clone),
                    templates,
                    localisation.unwrap_or(fallback),
                    recorded_language,
                    |name| {
                        let token = match &lexical.missing {
                            Some(reason) => Err(reason.clone()),
                            None => lexical.tokens.token_of(&frame, name),
                        };
                        recording_failure |= !matches!(token.as_ref(), Ok(Some(_)));
                        token
                    },
                )
                .and_then(|text| catalogue::plain_text(&text));
                let value = match resolved {
                    Ok(name) => (Some(name), None),
                    Err(reason) => {
                        let kind = if std::str::from_utf8(&row.name).is_err()
                            || (row.name.is_empty() && (template.is_err() || recording_failure))
                        {
                            GameScoreMissingKind::Recording
                        } else {
                            GameScoreMissingKind::Content
                        };
                        (None, Some(GameScoreMissing { kind, reason }))
                    }
                };
                (row.id, value)
            })
            .collect(),
    ))
}

/// A country's faction membership: known none, one membership row, or
/// unknown because a membership link is unresolved or names it twice.
enum Membership<'r> {
    NoFaction,
    Member(Cow<'r, FactionMemberRow>),
    Unknown,
}

/// The membership `unique_by_slot` gives `slot`.
fn membership<'r>(members: &SlotRows<'r, FactionMemberRow>, slot: u32) -> Membership<'r> {
    match members.rows.get(&slot) {
        None if members.unresolved_rows == 0 => Membership::NoFaction,
        Some(Some(member)) => Membership::Member(Cow::Borrowed(*member)),
        _ => Membership::Unknown,
    }
}

/// A slot's row by the rule of `unique_by_slot`: no row, one row, or several
/// rows whose raw references name the slot.
enum SlotRow<T> {
    Absent,
    One(T),
    Ambiguous,
}

impl<T> SlotRow<T> {
    fn one(self) -> Option<T> {
        match self {
            Self::One(row) => Some(row),
            Self::Absent | Self::Ambiguous => None,
        }
    }
}

/// [`SlotRow`] of one slot, from the raw link ids naming it and a complete
/// table keyed by its rows' raw country references.
fn slot_row<'r, R: RowCodec + Clone>(
    rows: Rowset<'r, R>,
    ids: &[u32],
) -> io::Result<SlotRow<Cow<'r, R>>> {
    let mut found = SlotRow::Absent;
    for id in ids {
        if let Some(row) = rows.get(u128::from(*id))? {
            found = match found {
                SlotRow::Absent => SlotRow::One(row),
                SlotRow::One(_) | SlotRow::Ambiguous => return Ok(SlotRow::Ambiguous),
            };
        }
    }
    Ok(found)
}

/// A country's map colour and why it is missing.
fn colour(
    frame: HourFrame<'_>,
    color: Option<[u8; 3]>,
) -> (Option<[u8; 3]>, Option<GameScoreMissing>) {
    let missing = color.is_none().then(|| {
        recording_missing(
            missing_reason(frame, TableId::CountryAppearance).unwrap_or_else(|| {
                "CountryAppearance has no unambiguous colour for this country".into()
            }),
        )
    });
    (color, missing)
}

/// The map colour of the CountryAppearance row of native country `country`, read by its key
/// from the rows the hour holds, as the shared style reader gives it.
fn appearance_colour(
    appearance: Rowset<'_, CountryAppearanceRow>,
    country: u32,
) -> io::Result<Option<[u8; 3]>> {
    Ok(appearance
        .get(u128::from(country))?
        .map(|row| row.map_color()))
}

fn orphan_input(raw: u32) -> (GameScoreMissingKind, String) {
    (
        GameScoreMissingKind::Recording,
        format!(
            "CountryGameScoreInputs country raw reference {raw} has no CountryLinks identity at this read"
        ),
    )
}

/// Every missing reason a country's facts note in an hour's warnings.
fn notes<'v>(
    values: &'v GameScoreValues,
    identity: &'v GameScoreIdentity,
) -> impl Iterator<Item = &'v GameScoreMissing> {
    values
        .amounts()
        .into_iter()
        .filter_map(|amount| amount.missing.as_ref())
        .chain(
            [
                identity.name_missing.as_ref(),
                identity.color_missing.as_ref(),
                identity.faction_missing.as_ref(),
                identity
                    .faction
                    .as_ref()
                    .and_then(|faction| faction.name_missing.as_ref()),
            ]
            .into_iter()
            .flatten(),
        )
}

fn faction(
    members: Option<Membership<'_>>,
    names: Option<&HashMap<u64, FactionName>>,
) -> (Option<GameScoreFaction>, Option<GameScoreMissing>) {
    let member = match members {
        None => {
            return (
                None,
                Some(recording_missing("FactionMembers is not complete")),
            );
        }
        Some(Membership::NoFaction) => return (None, None),
        Some(Membership::Member(member)) => member,
        Some(Membership::Unknown) => {
            return (
                None,
                Some(recording_missing(
                    "Faction membership has an unresolved or ambiguous country link",
                )),
            );
        }
    };
    let (name, name_missing) = match names.and_then(|rows| rows.get(&member.faction)) {
        Some(value) => value.clone(),
        None => {
            let reason = if names.is_none() {
                "Factions is not complete"
            } else {
                "Recorded faction membership names a missing Factions row"
            };
            (None, Some(recording_missing(reason)))
        }
    };
    (
        Some(GameScoreFaction {
            id: member.faction.to_string(),
            name,
            name_missing,
            ordinal: member.ordinal,
        }),
        None,
    )
}

pub struct Naming {
    localisation: Result<std::sync::Arc<Localisation>, String>,
    templates: Result<std::sync::Arc<Catalog>, String>,
    fallback: Localisation,
    warnings: Vec<String>,
}
impl Naming {
    pub fn load(source: &Source, context: &LoadContext) -> Self {
        let localisation = content::name_localisation(source, context.content.as_ref());
        let mut naming_warnings = Vec::new();
        let templates = context
            .content
            .as_ref()
            .ok_or_else(|| {
                "Faction naming content is unavailable until the game folders are chosen".to_owned()
            })
            .and_then(|options| {
                content::check_replay(
                    source.replay_ref(),
                    options.allow_unverified,
                    &mut naming_warnings,
                )
                .map_err(|error| format!("Faction naming content is unavailable: {error}"))?;
                source
                    .content(&options.options)
                    .map_err(|error| format!("Faction naming content is unavailable: {error}"))
            })
            .and_then(|layers| layers.layers.faction_templates());
        let viewer_language = context
            .content
            .as_ref()
            .and_then(|options| content::language(&options.options).ok());

        if let Ok(templates) = &templates {
            naming_warnings.extend(templates.warnings.iter().cloned());
        }
        let language = &source.replay_ref().identity.language;
        let mut fallback = Localisation::default();
        fallback.language = viewer_language
            .unwrap_or_else(|| language.strip_prefix("l_").unwrap_or(language).to_owned());
        Self {
            localisation,
            templates,
            fallback,
            warnings: naming_warnings,
        }
    }
    pub fn observer<'a>(&'a self, recorded_language: &'a str) -> ScoreObserver<'a> {
        ScoreObserver::new(
            self.localisation.as_deref().ok(),
            &self.fallback,
            self.templates.as_deref().map_err(Clone::clone),
            recorded_language,
        )
    }
}

/// What a native preparation reads of the recorded content, loaded once before its run: the
/// naming content, the defines' multipliers for hours whose capture was disabled, and the sort
/// hints with the warnings they make.
pub struct ScoreContent {
    naming: Naming,
    recorded_language: String,
    multipliers: GameScoreValues,
    hints: BTreeMap<String, String>,
    hint_warnings: BTreeMap<(GameScoreMissingKind, String), u64>,
}

impl ScoreContent {
    pub fn load(source: &Source, context: &LoadContext) -> Self {
        let naming = Naming::load(source, context);
        let language = &source.replay_ref().identity.language;
        let mut hint_warnings = BTreeMap::new();
        let hints = score_hints(naming.localisation.as_deref().ok(), &mut hint_warnings);
        Self {
            recorded_language: language.strip_prefix("l_").unwrap_or(language).to_owned(),
            multipliers: read_content_multipliers(source, context),
            naming,
            hints,
            hint_warnings,
        }
    }

    pub(crate) fn observer(&self) -> ScoreObserver<'_> {
        self.naming.observer(&self.recorded_language)
    }
}

fn read_content_multipliers(source: &Source, context: &LoadContext) -> GameScoreValues {
    let defines = context
        .content
        .as_ref()
        .ok_or_else(|| "Choose the recorded content folders to read score defines".to_owned())
        .and_then(|options| {
            catalogue::catalogue(source, options).map_err(|error| error.to_string())
        })
        .and_then(|cat| {
            installed_content_core::defines::read(&cat.content.layers).map_err(|error| error.to_string())
        });
    content_multipliers(defines.as_ref().map_err(String::as_str))
}

impl Product for GameScore {
    const KEY: &'static str = "game_score";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    type Reply = GameScoreReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn types() -> Vec<String> {
        vec![
            BranchParams::decl(),
            GameScoreMissingKind::decl(),
            GameScoreMissing::decl(),
            NativeFixed5Run::decl(),
            GameScoreSeries::decl(),
            GameScoreRun::<GameScoreIdentity>::decl(),
            GameScoreFaction::decl(),
            GameScoreIdentity::decl(),
            GameScoreCountry::decl(),
            GameScoreWarning::decl(),
            GameScoreReply::decl(),
            query::GameScoreStateReply::decl(),
        ]
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let replay = source.replay();
        let mut old_multipliers = None;
        let mut read_old_multipliers = || {
            old_multipliers
                .get_or_insert_with(|| read_content_multipliers(source, context))
                .clone()
        };
        let naming = Naming::load(source, context);
        let mut reply = scan(
            &replay,
            params,
            naming.localisation.as_deref().ok(),
            naming.templates.as_deref().map_err(Clone::clone),
            Some(&naming.fallback.language),
            &mut read_old_multipliers,
            cancelled,
        )?;
        reply.warnings.extend(naming.warnings);
        if dashboard_core::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
            return Err(oversized());
        }
        Ok(reply)
    }
}

fn scan(
    replay: &Replay,
    params: &BranchParams,
    localisation: Option<&Localisation>,
    templates: Result<&Catalog, String>,
    viewer_language: Option<&str>,
    old_multipliers: &mut impl FnMut() -> GameScoreValues,
    cancelled: &AtomicBool,
) -> io::Result<GameScoreReply> {
    if !replay
        .branches
        .iter()
        .any(|branch| branch.id == params.branch())
    {
        return Err(dashboard_core::invalid("game_score branch is not recorded"));
    }
    let selected = SCORE_TABLES;
    let mut countries = BTreeMap::<String, GameScoreCountry>::new();
    let mut reply = GameScoreReply {
        branch_id: params.branch_id.clone(),
        scale: Q,
        reads: Vec::new(),
        countries: Vec::new(),
        multipliers: GameScoreSeries::default(),
        reasons: Vec::new(),
        labels: score_labels(),
        hints: BTreeMap::new(),
        warnings: Vec::new(),
        score_warnings: Vec::new(),
    };
    let mut warnings = BTreeMap::<(GameScoreMissingKind, String), u64>::new();
    reply.hints = score_hints(localisation, &mut warnings);
    let mut reasons = ReasonTable::default();
    let mut budget = 0_usize;
    let language = &replay.identity.language;
    let recorded_language = language.strip_prefix("l_").unwrap_or(language);
    let mut empty_localisation = Localisation::default();
    empty_localisation.language = viewer_language.unwrap_or(recorded_language).to_owned();
    let mut observer = ScoreObserver::new(
        localisation,
        &empty_localisation,
        templates,
        recorded_language,
    );
    dashboard_core::blocks::scan_blocks(
        replay,
        replay
            .index
            .iter()
            .filter(|block| block.branch == params.branch()),
        selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                metadata::check_cancel(cancelled)?;
                let frames = match frames {
                    Ok(frames) => frames,
                    Err(error) => {
                        *warnings
                            .entry((
                                GameScoreMissingKind::Recording,
                                format!("Game score block unreadable: {error}"),
                            ))
                            .or_default() += 1;
                        continue;
                    }
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(dashboard_core::invalid("game_score request cancelled"));
                    }
                    let decoded = match Ok::<&DecodedFrame, std::io::Error>(&frame.decoded) {
                        Ok(decoded) => decoded,
                        Err(error) => {
                            *warnings
                                .entry((
                                    GameScoreMissingKind::Recording,
                                    format!("Game score frame unreadable: {error}"),
                                ))
                                .or_default() += 1;
                            break;
                        }
                    };
                    metadata::validate_campaign(decoded, replay)?;
                    reply.reads.push(frame.hour);
                    budget = budget
                        .checked_add(frame.hour.to_string().len())
                        .ok_or_else(oversized)?;
                    if budget > MAX_PRODUCT_REPLY_BYTES {
                        return Err(oversized());
                    }
                    let facts = observer.observe(decoded, replay.format, old_multipliers)?;
                    reply.multipliers.push(
                        frame.hour,
                        &facts.multipliers,
                        false,
                        &mut reasons,
                        &mut budget,
                    )?;
                    for (tag, (values, identity)) in &facts.countries {
                        let series =
                            countries
                                .entry(tag.clone())
                                .or_insert_with(|| GameScoreCountry {
                                    tag: tag.clone(),
                                    identity: Vec::new(),
                                    scores: GameScoreSeries::default(),
                                });
                        let same_identity = series.identity.last().is_some_and(|run| {
                            run.last_hour.checked_add(1) == Some(frame.hour)
                                && run.value.canonical_slot == identity.canonical_slot
                                && run.value.native_country == identity.native_country
                        });
                        series.scores.push(
                            frame.hour,
                            values,
                            !same_identity,
                            &mut reasons,
                            &mut budget,
                        )?;
                        push_run(
                            &mut series.identity,
                            frame.hour,
                            identity.clone(),
                            false,
                            &mut budget,
                        )?;
                    }
                    for (key, count) in &facts.warnings {
                        *warnings.entry(key.clone()).or_default() += count;
                    }
                }
            }
            Ok(())
        },
    )?;
    reply.countries = countries.into_values().collect();
    reply.reasons = reasons.entries;
    reply.score_warnings = warnings
        .into_iter()
        .map(|((kind, reason), observations)| GameScoreWarning {
            kind,
            reason,
            observations,
        })
        .collect();
    if dashboard_core::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(oversized());
    }
    Ok(reply)
}

/// Every table the score observer and its naming read.
pub const SCORE_TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryGameScoreInputs,
    TableId::AirWingGameScoreStats,
    TableId::GameScoreMultipliers,
    TableId::CountryFactories,
    TableId::AirBaseRegistrations,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::LexicalTokens,
];

/// One hour's score tables as the observer reads them, and how it tells a table unchanged since
/// an hour it held.
pub(crate) type ScoreHour<'f> = Viewed<'f>;

/// A table that was not complete as an hour held it: a decoded view, told unchanged by its
/// rows; or a walked table's epoch.
enum Incomplete {
    View(SharedView),
    Epoch(Option<u64>),
}

/// `table` as [`Held::incomplete`] keeps it, when it is not complete.
fn incomplete(hour: ScoreHour<'_>, table: TableId) -> Option<Incomplete> {
    match (hour.epochs, hour.view(table)) {
        (None, Some(view)) => {
            (!matches!(**view, TableView::Complete(_))).then(|| Incomplete::View(view.clone()))
        }
        (epochs, _) => (!matches!(hour.frame.status(table), OperandStatus::Complete))
            .then(|| Incomplete::Epoch(epochs.and_then(|epochs| epochs.get(table)))),
    }
}

/// Whether `hour`'s `table` holds what `before`, kept while it was not complete, held.
fn unchanged(hour: ScoreHour<'_>, table: TableId, before: &Incomplete) -> bool {
    match (before, hour.epochs) {
        (Incomplete::View(before), None) => hour.view(table).is_some_and(|now| now == before),
        (Incomplete::Epoch(epoch), Some(epochs)) => epochs.same(table, *epoch),
        _ => false,
    }
}

/// The production frame calculation, shared by history replies and preparation.
pub struct ScoreObserver<'a> {
    localisation: Option<&'a Localisation>,
    fallback: &'a Localisation,
    templates: Result<&'a Catalog, String>,
    recorded_language: &'a str,
    lexical: Option<FactionLexical>,
    names: Option<HourNames<'a>>,
    previous: Option<(i32, Vec<Mark>)>,
    country_keys: Option<dashboard_text::metadata::CountryKeys>,
    current: Option<FrameScores>,
    /// What the current facts were read from, so that the next hour can be
    /// read from its changed rows.
    held: Option<Held>,
    /// Read every changed hour from the whole tables, never from changed rows.
    #[cfg(test)]
    pub(super) whole_tables: bool,
    /// Countries whose facts were computed.
    #[cfg(test)]
    pub(super) computed: usize,
    /// Hours read from their changed rows.
    #[cfg(test)]
    pub(super) row_hours: usize,
}
pub(crate) struct FrameScores {
    projected: i32,
    multipliers: GameScoreValues,
    countries: BTreeMap<String, (GameScoreValues, GameScoreIdentity)>,
    warnings: BTreeMap<(GameScoreMissingKind, String), u64>,
}

/// What an observed hour changed in the facts.
pub(crate) enum Change {
    /// Nothing: the facts are the previous hour's.
    Same,
    /// Every country was read again; any may have changed, appeared or gone.
    All,
    /// Only these countries' facts and these warnings' counts changed. The
    /// countries present and the multipliers are the previous hour's.
    Countries {
        tags: Vec<String>,
        warnings: Vec<(GameScoreMissingKind, String)>,
    },
}

/// Fields each part of the facts reads, by table. A changed row that changes
/// none of them, and adds no row, reads nothing again.
static HOLDER_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::Countries, &["canonical_slot", "tag"]));
static INPUT_FIELDS: LazyLock<u64> = LazyLock::new(|| {
    field_mask(
        TableId::CountryGameScoreInputs,
        &[
            "army_count",
            "army_error",
            "navy_count",
            "navy_error",
            "land_count",
            "land_error",
            "air_state",
            "air_error",
            "first_sites",
        ],
    )
});
static FACTORY_FIELDS: LazyLock<u64> = LazyLock::new(|| {
    field_mask(
        TableId::CountryFactories,
        &[
            "naval_total_raw",
            "civilian_total_raw",
            "military_total_raw",
            "civilian_to_others",
            "civilian_to_overlord",
            "military_to_others",
            "military_to_overlord",
        ],
    )
});
static WING_FIELDS: LazyLock<u64> = LazyLock::new(|| {
    field_mask(
        TableId::AirWingGameScoreStats,
        &["planes", "planes_error", "stat_raw", "stat_error"],
    )
});
static REGISTRATION_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::AirBaseRegistrations, &["wings"]));
static COLOUR_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::CountryAppearance, &["red", "green", "blue"]));
/// The naming operands `same_name_rows` compares.
static APPEARANCE_NAME_FIELDS: LazyLock<u64> = LazyLock::new(|| {
    field_mask(
        TableId::CountryAppearance,
        &["cosmetic_tag", "name_tag", "leader_ideology", "overlord"],
    )
});
static RULING_PARTY_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::Politics, &["ruling_party"]));
static PARTY_TOKEN_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::PoliticalParties, &["token"]));
static AUTONOMY_FIELDS: LazyLock<u64> = LazyLock::new(|| field_mask(TableId::Autonomy, &["state"]));
static FACTION_NAME_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::Factions, &["name", "template_token"]));
const LINK_FIELDS: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
const MEMBER_FIELDS: u64 = mask::<FactionMemberRow>(&["faction", "ordinal"]);
static MULTIPLIER_FIELDS: LazyLock<u64> = LazyLock::new(|| {
    field_mask(
        TableId::GameScoreMultipliers,
        &[
            "total_raw",
            "total_error",
            "army_raw",
            "army_error",
            "navy_raw",
            "navy_error",
            "air_raw",
            "air_error",
            "industry_raw",
            "industry_error",
            "land_raw",
            "land_error",
        ],
    )
});

/// The tables besides Countries and CountryLinks whose changed rows an hour
/// is read from.
const ROW_TABLES: [TableId; 12] = [
    TableId::GameScoreMultipliers,
    TableId::CountryGameScoreInputs,
    TableId::CountryFactories,
    TableId::AirBaseRegistrations,
    TableId::AirWingGameScoreStats,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::FactionMembers,
    TableId::Factions,
    TableId::LexicalTokens,
];

/// A table's changed rows at an hour, each its key and the fields it wrote, and the keys of
/// its deleted rows.
type Rows<'e> = (&'e [(u128, u64)], &'e [u128]);

/// Whether a changed row is new, or changes a field in `fields`: a new row writes every field.
fn reads(mask: u64, fields: u64) -> bool {
    mask & fields != 0
}

/// Whether a table's changed rows add, remove or change a field in `fields`.
fn touches((changed, deleted): Rows<'_>, fields: u64) -> bool {
    !deleted.is_empty() || changed.iter().any(|&(_, mask)| reads(mask, fields))
}

/// One table's changes at an hour, as [`Edits`] holds them.
#[derive(Default)]
pub(crate) struct Edited {
    /// Its rows are the hour before's changed by these alone.
    pub(crate) exact: bool,
    /// The hour changed, added or removed any of its rows, or changed its status.
    pub(crate) moved: bool,
    /// Each changed row's key and the fields it wrote, every field for a new row.
    pub(crate) changed: Vec<(u128, u64)>,
    pub(crate) deleted: Vec<u128>,
}

/// An hour's changes to [`SCORE_TABLES`], by their position there.
pub(crate) struct Edits {
    pub(crate) tables: Vec<Edited>,
}

impl Edits {
    pub(crate) fn new() -> Self {
        Self {
            tables: SCORE_TABLES.iter().map(|_| Edited::default()).collect(),
        }
    }

    fn of(&self, table: TableId) -> Option<&Edited> {
        let at = SCORE_TABLES.iter().position(|held| *held == table)?;
        self.tables.get(at)
    }

    /// `table`'s changed rows, when its rows are the hour before's changed by them alone.
    fn rows(&self, table: TableId) -> Option<Rows<'_>> {
        self.of(table)
            .filter(|edited| edited.exact)
            .map(|edited| (edited.changed.as_slice(), edited.deleted.as_slice()))
    }

    fn moved(&self, table: TableId) -> bool {
        self.of(table).is_none_or(|edited| edited.moved)
    }
}

/// Whether a slot's name operands, or those of an overlord its name can
/// reach, differ between two reads. A name reads nothing else.
fn chain_changed(before: &CountryNames<'_>, after: &CountryNames<'_>, slot: u32) -> bool {
    let mut next = Some(slot);
    let mut visited = BTreeSet::new();
    while let Some(slot) = next {
        if !visited.insert(slot) {
            return false;
        }
        let operands = after.operands(slot);
        if before.operands(slot) != operands {
            return true;
        }
        next = operands
            .and_then(|operands| operands.as_ref().ok())
            .and_then(|operands| operands.overlord);
    }
    false
}

/// The operands the current facts were read from, by what reads them, so that
/// an hour's changed rows name the countries whose facts they change.
struct Held {
    branch: BranchId,
    hour: i32,
    countries: usize,
    /// Found tags by canonical slot.
    slot_tags: HashMap<u32, Vec<String>>,
    /// Found tags by the native id of the Countries row their colour is read from.
    colour_tags: HashMap<u32, Vec<String>>,
    /// The raw link ids naming each canonical slot.
    link_ids: HashMap<u32, Vec<u32>>,
    /// Raw references of score input rows that no link names.
    orphans: HashSet<u32>,
    /// Each registration's wings, and the slots registering each wing.
    registrations: HashMap<(u64, u32), Vec<u64>>,
    wing_slots: HashMap<u64, HashMap<u32, usize>>,
    /// Faction names, when Factions is complete.
    factions: Option<HashMap<u64, FactionName>>,
    /// Raw references of membership rows that no link names.
    unresolved_members: HashSet<u32>,
    /// Each of `ROW_TABLES` that was not complete, as it was. Such a table is
    /// read whole every hour, so it has no changed rows, and is unchanged
    /// when it holds the same.
    incomplete: [Option<Incomplete>; ROW_TABLES.len()],
}

/// The raw country each row of a complete table keyed by it names, from its keys alone.
fn raw_countries<R: RowCodec>(rows: Option<Rowset<'_, R>>) -> impl Iterator<Item = u32> + '_ {
    rows.into_iter().flat_map(|rows| rows.keys()).map(|key| key as u32)
}

impl Held {
    fn read(
        hour: ScoreHour<'_>,
        countries: usize,
        links: &[CountryLinkRow],
        slot_tags: HashMap<u32, Vec<String>>,
        colour_tags: HashMap<u32, Vec<String>>,
        factions: Option<HashMap<u64, FactionName>>,
    ) -> io::Result<Self> {
        let frame = hour.frame;
        let mut link_ids = HashMap::<u32, Vec<u32>>::new();
        for link in links {
            link_ids
                .entry(link.canonical_slot)
                .or_default()
                .push(link.id);
        }
        let unlinked = |raw: &u32| metadata::link_slot(links, *raw).is_none();
        let mut held = Self {
            branch: frame.branch(),
            hour: frame.hour(),
            countries,
            slot_tags,
            colour_tags,
            link_ids,
            // Each table is keyed by the raw country its rows name.
            orphans: raw_countries(frame.complete::<tables::CountryGameScoreInputs>(0).ok())
                .filter(unlinked)
                .collect(),
            registrations: HashMap::new(),
            wing_slots: HashMap::new(),
            factions,
            unresolved_members: raw_countries(frame.complete::<tables::FactionMembers>(0).ok())
                .filter(unlinked)
                .collect(),
            incomplete: ROW_TABLES.map(|table| incomplete(hour, table)),
        };
        if let Ok(rows) = frame.complete::<tables::AirBaseRegistrations>(*REGISTRATION_FIELDS) {
            for row in rows.iter() {
                let row = row?;
                held.register((row.base, row.slot), row.wings.clone());
            }
        }
        Ok(held)
    }

    /// Index a registration's wings in place of those it listed before; no
    /// wings remove it.
    fn register(&mut self, key: (u64, u32), wings: Vec<u64>) {
        for wing in self.registrations.remove(&key).unwrap_or_default() {
            if let Some(slots) = self.wing_slots.get_mut(&wing) {
                if let Some(count) = slots.get_mut(&key.1) {
                    *count -= 1;
                    if *count == 0 {
                        slots.remove(&key.1);
                    }
                }
                if slots.is_empty() {
                    self.wing_slots.remove(&wing);
                }
            }
        }
        for &wing in &wings {
            *self
                .wing_slots
                .entry(wing)
                .or_default()
                .entry(key.1)
                .or_default() += 1;
        }
        if !wings.is_empty() {
            self.registrations.insert(key, wings);
        }
    }
}

/// The parts of one country's facts an hour's changed rows reach.
#[derive(Default)]
struct Parts {
    values: bool,
    name: bool,
    faction: bool,
    /// The native country whose appearance row holds its colour.
    colour: Option<u32>,
}

impl<'a> ScoreObserver<'a> {
    pub(crate) fn new(
        localisation: Option<&'a Localisation>,
        fallback: &'a Localisation,
        templates: Result<&'a Catalog, String>,
        recorded_language: &'a str,
    ) -> Self {
        Self {
            localisation,
            fallback,
            templates,
            recorded_language,
            lexical: None,
            names: None,
            previous: None,
            country_keys: None,
            current: None,
            held: None,
            #[cfg(test)]
            whole_tables: false,
            #[cfg(test)]
            computed: 0,
            #[cfg(test)]
            row_hours: 0,
        }
    }
    pub(crate) fn observe(
        &mut self,
        decoded: &DecodedFrame,
        format: FormatVersion,
        old_multipliers: &mut impl FnMut() -> GameScoreValues,
    ) -> io::Result<&FrameScores> {
        self.observe_changed(ScoreHour::decoded(decoded), format, old_multipliers, None)
            .map(|(facts, _)| facts)
    }

    /// Whether names reach slot zero, whose refusal names its hour: every hour a score table
    /// changes in is then read whole.
    pub(crate) fn names_hourly(&self) -> bool {
        self.names.as_ref().is_some_and(|names| names.hourly)
    }

    /// The hour's facts read whole.
    pub(crate) fn observe_whole(
        &mut self,
        hour: ScoreHour<'_>,
        format: FormatVersion,
        old_multipliers: &mut impl FnMut() -> GameScoreValues,
    ) -> io::Result<&FrameScores> {
        let samples = sample(hour.frame, format, old_multipliers)?;
        self.present(hour, samples)
    }

    /// The facts of an hour that changes nothing they read, going on from the held hour, the
    /// hour before; false when there is none, and the hour is read whole.
    pub(crate) fn advance(&mut self, hour: i32) -> bool {
        let (Some(held), Some((previous, _)), Some(_)) =
            (&mut self.held, &mut self.previous, &self.current)
        else {
            return false;
        };
        if held.hour.checked_add(1) != Some(hour) || previous.checked_add(1) != Some(hour) {
            return false;
        }
        held.hour = hour;
        *previous = hour;
        true
    }

    /// The hour's facts and what changed in them. With the hour's changed
    /// rows, only the countries whose read rows changed are computed again.
    pub(crate) fn observe_changed(
        &mut self,
        hour: ScoreHour<'_>,
        format: FormatVersion,
        old_multipliers: &mut impl FnMut() -> GameScoreValues,
        changes: Option<&Edits>,
    ) -> io::Result<(&FrameScores, Change)> {
        if let Some(changes) = changes
            && let Some(change) = self.observe_rows(hour, format, changes)?
        {
            let facts = self.current.as_ref().expect("facts read from changed rows");
            return Ok((facts, change));
        }
        if self.reuses(hour) {
            let previous = self.previous.as_mut().expect("previous score read");
            previous.0 = hour.hour();
            if let Some(index) = SCORE_TABLES
                .iter()
                .position(|table| *table == TableId::Countries)
            {
                previous.1[index] = hour.mark(TableId::Countries);
            }
            if let Some(held) = &mut self.held {
                held.hour = hour.hour();
            }
            let facts = self.current.as_ref().expect("previous score facts");
            return Ok((facts, Change::Same));
        }
        let samples = sample(hour.frame, format, old_multipliers)?;
        Ok((self.present(hour, samples)?, Change::All))
    }

    /// Whether every table the facts read is the previous hour's; Countries
    /// need only hold the same country keys.
    fn reuses(&self, hour: ScoreHour<'_>) -> bool {
        self.previous.as_ref().is_some_and(|(at, marks)| {
            at.checked_add(1) == Some(hour.hour())
                && marks.iter().zip(SCORE_TABLES).all(|(before, table)| {
                    hour.is(*table, before)
                        || (*table == TableId::Countries
                            && self.names.as_ref().is_some_and(|names| !names.hourly)
                            && hour
                                .frame
                                .complete::<tables::Countries>(*HOLDER_FIELDS)
                                .ok()
                                .is_some_and(|rows| {
                                    self.country_keys
                                        .as_ref()
                                        .is_some_and(|keys| keys.same_in(rows))
                                }))
                })
        })
    }

    /// Read the hour from its changed rows alone: possible when it continues
    /// the held hour with the same countries, links and multipliers, every
    /// changed table it reads changed by one recorded delta, and no name
    /// reaches slot zero. Computes again only the parts of the countries
    /// whose read rows changed. None when the whole tables are needed.
    fn observe_rows(
        &mut self,
        hour: ScoreHour<'_>,
        format: FormatVersion,
        changes: &Edits,
    ) -> io::Result<Option<Change>> {
        #[cfg(test)]
        if self.whole_tables {
            return Ok(None);
        }
        let frame = hour.frame;
        let (Some(held), Some(names), Some(_)) = (&self.held, &self.names, &self.current) else {
            return Ok(None);
        };
        if held.branch != frame.branch()
            || held.hour.checked_add(1) != Some(frame.hour())
            || names.hourly
            || changes.moved(TableId::CountryLinks)
        {
            return Ok(None);
        }
        let Some(countries) = changes.rows(TableId::Countries) else {
            return Ok(None);
        };
        if touches(countries, *HOLDER_FIELDS) {
            return Ok(None);
        }
        let (Ok(country_rows), Ok(links)) = (
            frame.complete::<tables::Countries>(*HOLDER_FIELDS),
            frame.complete::<tables::CountryLinks>(LINK_FIELDS),
        ) else {
            return Ok(None);
        };
        if country_rows.len() != held.countries {
            return Ok(None);
        }
        let mut rows: [Rows<'_>; ROW_TABLES.len()] = [(&[], &[]); ROW_TABLES.len()];
        for ((rows, table), before) in rows.iter_mut().zip(ROW_TABLES).zip(&held.incomplete) {
            *rows = match changes.rows(table) {
                Some(changed) => changed,
                None if before
                    .as_ref()
                    .is_some_and(|before| unchanged(hour, table, before)) =>
                {
                    (&[], &[])
                }
                None => return Ok(None),
            };
        }
        let [
            multipliers,
            inputs,
            factories,
            registrations,
            wings,
            appearance,
            politics,
            parties,
            autonomy,
            members,
            factions,
            tokens,
        ] = rows;
        if touches(multipliers, u64::MAX) {
            return Ok(None);
        }
        let fresh_names = if touches(appearance, *APPEARANCE_NAME_FIELDS)
            || touches(politics, *RULING_PARTY_FIELDS)
            || touches(parties, *PARTY_TOKEN_FIELDS)
            || touches(autonomy, *AUTONOMY_FIELDS)
        {
            let fresh = HourNames::read(
                hour,
                slice(country_rows)?.as_ref(),
                self.localisation.unwrap_or(self.fallback),
            );
            // Names that reach slot zero are read whole, every hour.
            if fresh.hourly {
                return Ok(None);
            }
            Some(fresh)
        } else {
            None
        };
        #[cfg(test)]
        {
            self.row_hours += 1;
        }
        self.previous = Some((
            frame.hour(),
            SCORE_TABLES.iter().map(|&table| hour.mark(table)).collect(),
        ));
        let Self {
            localisation,
            fallback,
            templates,
            recorded_language,
            lexical,
            names,
            current,
            held,
            ..
        } = self;
        let held = held.as_mut().expect("held score operands");
        held.hour = frame.hour();
        held.incomplete = ROW_TABLES.map(|table| incomplete(hour, table));
        // A change to Countries alone, without a holder change, reads nothing.
        if !SCORE_TABLES
            .iter()
            .any(|table| *table != TableId::Countries && changes.moved(*table))
        {
            return Ok(Some(Change::Same));
        }
        let link_slot = |raw: u32| metadata::link_slot_in(&links, raw);
        let mut name_slots = HashSet::new();
        if let Some(fresh) = fresh_names {
            let before = names.as_ref().expect("held names");
            name_slots.extend(
                held.slot_tags
                    .keys()
                    .copied()
                    .filter(|slot| chain_changed(&before.names, &fresh.names, *slot)),
            );
            *names = Some(fresh);
        }
        let names = names.as_mut().expect("held names");
        let mut value_slots = HashSet::new();
        let mut faction_slots = HashSet::new();
        let mut noted = BTreeMap::<(GameScoreMissingKind, String), i64>::new();
        for &(key, mask) in inputs.0 {
            let raw = key as u32;
            match link_slot(raw) {
                Some(slot) => {
                    if reads(mask, *INPUT_FIELDS) {
                        value_slots.insert(slot);
                    }
                }
                None => {
                    if held.orphans.insert(raw) {
                        *noted.entry(orphan_input(raw)).or_default() += 1;
                    }
                }
            }
        }
        for &key in inputs.1 {
            let raw = key as u32;
            match link_slot(raw) {
                Some(slot) => {
                    value_slots.insert(slot);
                }
                None => {
                    if held.orphans.remove(&raw) {
                        *noted.entry(orphan_input(raw)).or_default() -= 1;
                    }
                }
            }
        }
        for raw in factories
            .0
            .iter()
            .filter(|&&(_, mask)| reads(mask, *FACTORY_FIELDS))
            .map(|&(key, _)| key)
            .chain(factories.1.iter().copied())
        {
            value_slots.extend(link_slot(raw as u32));
        }
        let registered = frame
            .complete::<tables::AirBaseRegistrations>(*REGISTRATION_FIELDS)
            .ok();
        for &(key, mask) in registrations.0 {
            if !reads(mask, *REGISTRATION_FIELDS) {
                continue;
            }
            let at = ((key >> 32) as u64, key as u32);
            let wings = match registered {
                Some(rows) => rows
                    .get(u128::from(at.0) << 32 | u128::from(at.1))?
                    .map(|row| row.wings.clone())
                    .unwrap_or_default(),
                None => Vec::new(),
            };
            held.register(at, wings);
            value_slots.insert(at.1);
        }
        for &key in registrations.1 {
            let key = ((key >> 32) as u64, key as u32);
            held.register(key, Vec::new());
            value_slots.insert(key.1);
        }
        for wing in wings
            .0
            .iter()
            .filter(|&&(_, mask)| reads(mask, *WING_FIELDS))
            .map(|&(key, _)| key)
            .chain(wings.1.iter().copied())
        {
            if let Some(slots) = held.wing_slots.get(&(wing as u64)) {
                value_slots.extend(slots.keys().copied());
            }
        }
        let colour_ids: HashSet<u32> = appearance
            .0
            .iter()
            .filter(|&&(_, mask)| reads(mask, *COLOUR_FIELDS))
            .map(|&(key, _)| key as u32)
            .chain(appearance.1.iter().map(|key| *key as u32))
            .collect();
        let unresolved = !held.unresolved_members.is_empty();
        for (raw, present) in members
            .0
            .iter()
            .map(|&(key, _)| (key as u32, true))
            .chain(members.1.iter().map(|key| (*key as u32, false)))
        {
            match link_slot(raw) {
                Some(slot) => {
                    faction_slots.insert(slot);
                }
                None if present => {
                    held.unresolved_members.insert(raw);
                }
                None => {
                    held.unresolved_members.remove(&raw);
                }
            }
        }
        // Known no-faction needs every membership link resolved.
        let mut all_factions = unresolved == held.unresolved_members.is_empty();
        if touches(factions, *FACTION_NAME_FIELDS) || touches(tokens, u64::MAX) {
            if lexical.as_ref().is_none_or(|previous| !previous.holds(hour)) {
                *lexical = Some(FactionLexical::new(hour));
            }
            let fresh = match lexical.as_ref() {
                Some(lexical) => faction_names_at(
                    frame,
                    *localisation,
                    fallback,
                    recorded_language,
                    templates.as_ref().copied().map_err(Clone::clone),
                    lexical,
                )?,
                None => None,
            };
            match (&held.factions, &fresh) {
                (Some(before), Some(after)) => {
                    let renamed: HashSet<u64> = before
                        .keys()
                        .chain(after.keys())
                        .filter(|id| before.get(id) != after.get(id))
                        .copied()
                        .collect();
                    if !renamed.is_empty()
                        && let Ok(rows) = frame.complete::<tables::FactionMembers>(MEMBER_FIELDS)
                    {
                        for row in rows.iter() {
                            let row = row?;
                            if renamed.contains(&row.faction) {
                                faction_slots.extend(link_slot(row.country));
                            }
                        }
                    }
                }
                (None, None) => {}
                _ => all_factions = true,
            }
            held.factions = fresh;
        }
        let held = &*held;
        let mut dirty = BTreeMap::<&str, Parts>::new();
        let mut reach = |slots: &mut dyn Iterator<Item = u32>, part: fn(&mut Parts)| {
            for slot in slots {
                for tag in held.slot_tags.get(&slot).into_iter().flatten() {
                    part(dirty.entry(tag.as_str()).or_default());
                }
            }
        };
        reach(&mut value_slots.into_iter(), |parts| parts.values = true);
        reach(&mut name_slots.into_iter(), |parts| parts.name = true);
        if all_factions {
            reach(&mut held.slot_tags.keys().copied(), |parts| {
                parts.faction = true;
            });
        } else {
            reach(&mut faction_slots.into_iter(), |parts| parts.faction = true);
        }
        for id in colour_ids {
            for tag in held.colour_tags.get(&id).into_iter().flatten() {
                dirty.entry(tag.as_str()).or_default().colour = Some(id);
            }
        }
        let current = current.as_mut().expect("held score facts");
        let multipliers = current.multipliers.clone();
        let input_refusal = format.table_refusal(TableId::CountryGameScoreInputs);
        let context = ScoreContext {
            frame,
            multipliers: &multipliers,
            input_refusal: input_refusal.as_deref(),
            wings: frame
                .complete::<tables::AirWingGameScoreStats>(*WING_FIELDS)
                .ok(),
            registrations: registered,
        };
        let input_rows = frame
            .complete::<tables::CountryGameScoreInputs>(*INPUT_FIELDS)
            .ok();
        let factory_rows = frame
            .complete::<tables::CountryFactories>(*FACTORY_FIELDS)
            .ok();
        let member_rows = frame
            .complete::<tables::FactionMembers>(MEMBER_FIELDS)
            .ok();
        let appearance_rows = if dirty.values().any(|parts| parts.colour.is_some()) {
            Some(recorded::<tables::CountryAppearance>(frame, *COLOUR_FIELDS)?)
        } else {
            None
        };
        let mut changed = Vec::new();
        #[cfg(test)]
        let mut computed = 0;
        for (tag, parts) in dirty {
            let Some(before) = current.countries.get(tag) else {
                continue;
            };
            let Some(slot) = before.1.canonical_slot else {
                continue;
            };
            #[cfg(test)]
            {
                computed += 1;
            }
            let ids = held.link_ids.get(&slot).map_or(&[][..], Vec::as_slice);
            let (mut values, mut identity) = before.clone();
            if parts.values {
                let input = match input_rows {
                    Some(rows) => slot_row(rows, ids)?.one(),
                    None => None,
                };
                let factory = match factory_rows {
                    Some(rows) => slot_row(rows, ids)?.one(),
                    None => None,
                };
                values = context.scores(input.as_deref(), slot, factory.as_deref())?;
            }
            if parts.name {
                (identity.name, identity.name_missing) = names.name(slot);
            }
            if let (Some(country), Some(appearance)) = (parts.colour, appearance_rows) {
                (identity.color, identity.color_missing) =
                    colour(frame, appearance_colour(appearance, country)?);
            }
            if parts.faction {
                let membership = match member_rows {
                    Some(rows) => Some(match slot_row(rows, ids)? {
                        SlotRow::Absent if held.unresolved_members.is_empty() => {
                            Membership::NoFaction
                        }
                        SlotRow::One(member) => Membership::Member(member),
                        SlotRow::Absent | SlotRow::Ambiguous => Membership::Unknown,
                    }),
                    None => None,
                };
                (identity.faction, identity.faction_missing) =
                    faction(membership, held.factions.as_ref());
            }
            if (&values, &identity) == (&before.0, &before.1) {
                continue;
            }
            for missing in notes(&before.0, &before.1) {
                *noted
                    .entry((missing.kind.clone(), missing.reason.clone()))
                    .or_default() -= 1;
            }
            for missing in notes(&values, &identity) {
                *noted
                    .entry((missing.kind.clone(), missing.reason.clone()))
                    .or_default() += 1;
            }
            current.countries.insert(tag.to_owned(), (values, identity));
            changed.push(tag.to_owned());
        }
        let mut warnings = Vec::new();
        for (key, delta) in noted {
            if delta == 0 {
                continue;
            }
            let count = current.warnings.entry(key.clone()).or_default();
            *count = count
                .checked_add_signed(delta)
                .ok_or_else(|| dashboard_core::invalid("game_score warning count underflow"))?;
            if *count == 0 {
                current.warnings.remove(&key);
            }
            warnings.push(key);
        }
        current.projected = frame.hour();
        #[cfg(test)]
        {
            self.computed += computed;
        }
        Ok(Some(Change::Countries {
            tags: changed,
            warnings,
        }))
    }

    /// One hour can prove its appearance, but cannot prove prior race continuity.
    fn present(&mut self, hour: ScoreHour<'_>, samples: ScoreSamples) -> io::Result<&FrameScores> {
        let frame = hour.frame;
        let ScoreSamples {
            multipliers,
            countries: mut sampled,
            warnings: mut frame_warnings,
        } = samples;
        let mut countries = BTreeMap::new();
        let (Some(country_rows), Some(links)) = (
            complete::<tables::Countries>(frame, *HOLDER_FIELDS)?,
            complete::<tables::CountryLinks>(frame, LINK_FIELDS)?,
        ) else {
            self.previous = None;
            self.held = None;
            self.current = Some(FrameScores {
                projected: frame.hour(),
                multipliers,
                countries,
                warnings: frame_warnings,
            });
            return Ok(self.current.as_ref().expect("score facts"));
        };
        let (country_rows, links) = (&*country_rows, &*links);
        self.country_keys = Some(dashboard_text::metadata::CountryKeys::read(country_rows));
        let member_rows = complete::<tables::FactionMembers>(frame, MEMBER_FIELDS)?;
        let members = member_rows
            .as_deref()
            .map(|rows| unique_by_slot(rows, links, |row| row.country));
        if self
            .lexical
            .as_ref()
            .is_none_or(|previous| !previous.holds(hour))
        {
            self.lexical = Some(FactionLexical::new(hour));
        }
        let factions = match self.lexical.as_ref() {
            Some(lexical) => faction_names_at(
                frame,
                self.localisation,
                self.fallback,
                self.recorded_language,
                self.templates.as_ref().copied().map_err(Clone::clone),
                lexical,
            )?,
            None => None,
        };
        // The colours of the rows the hour holds, read as the shared style reader reads them.
        let appearance = recorded::<tables::CountryAppearance>(frame, *COLOUR_FIELDS)?;
        if self.names.as_ref().is_none_or(|read| !read.agrees(hour)) {
            self.names = Some(HourNames::read(
                hour,
                country_rows,
                self.localisation.unwrap_or(self.fallback),
            ));
        }
        let names = self.names.as_mut().expect("names agree with this hour");
        // A tag's colour is read from the first Countries row with its
        // (lossy) tag, as the shared style reader keys them.
        let mut style_rows = HashMap::<Cow<'_, str>, u32>::new();
        for row in country_rows {
            style_rows
                .entry(String::from_utf8_lossy(&row.tag))
                .or_insert(row.id);
        }
        let mut slot_tags = HashMap::<u32, Vec<String>>::new();
        let mut colour_tags = HashMap::<u32, Vec<String>>::new();
        for (tag, country) in tagged_countries(country_rows, links) {
            let tag = match std::str::from_utf8(tag) {
                Ok(tag) => tag.to_owned(),
                Err(_) => continue,
            };
            let (values, _) = sampled
                .remove(&tag)
                .expect("score sampler covers tagged countries");
            let identity = match country {
                TaggedCountry::Found(country, slot) => {
                    slot_tags.entry(slot).or_default().push(tag.clone());
                    let style = style_rows.get(tag.as_str()).copied();
                    if let Some(id) = style {
                        colour_tags.entry(id).or_default().push(tag.clone());
                    }
                    let (faction, faction_missing) = faction(
                        members.as_ref().map(|members| membership(members, slot)),
                        factions.as_ref(),
                    );
                    let (name, name_missing) = names.name(slot);
                    let color = match style {
                        Some(id) => appearance_colour(appearance, id)?,
                        None => None,
                    };
                    let (color, color_missing) = colour(frame, color);
                    GameScoreIdentity {
                        canonical_slot: Some(slot),
                        native_country: Some(country.id),
                        name,
                        name_missing,
                        color,
                        color_missing,
                        faction,
                        faction_missing,
                    }
                }
                _ => {
                    let missing =
                        recording_missing("Country identity is unresolved by CountryLinks");
                    GameScoreIdentity {
                        canonical_slot: None,
                        native_country: None,
                        name: None,
                        name_missing: Some(missing.clone()),
                        color: None,
                        color_missing: Some(missing.clone()),
                        faction: None,
                        faction_missing: Some(missing),
                    }
                }
            };
            for missing in [
                identity.name_missing.as_ref(),
                identity.color_missing.as_ref(),
                identity.faction_missing.as_ref(),
                identity
                    .faction
                    .as_ref()
                    .and_then(|faction| faction.name_missing.as_ref()),
            ]
            .into_iter()
            .flatten()
            {
                note_missing(Some(missing), &mut frame_warnings);
            }
            countries.insert(tag, (values, identity));
        }
        #[cfg(test)]
        {
            self.computed += countries.len();
        }
        self.previous = Some((
            frame.hour(),
            SCORE_TABLES.iter().map(|&table| hour.mark(table)).collect(),
        ));
        self.held = Some(Held::read(
            hour,
            country_rows.len(),
            links,
            slot_tags,
            colour_tags,
            factions,
        )?);
        self.current = Some(FrameScores {
            projected: frame.hour(),
            multipliers,
            countries,
            warnings: frame_warnings,
        });
        Ok(self.current.as_ref().expect("score facts"))
    }
}

/// Names read at the last hour whose naming operands changed, each resolved
/// once. Holding that hour's views copies each at most once, at its next update.
struct HourNames<'a> {
    /// The names reach slot zero: they are read every hour.
    hourly: bool,
    /// What each of `country_names::TABLES` was when they were read; none when hourly.
    read: Vec<Incomplete>,
    names: CountryNames<'a>,
    resolved: HashMap<u32, (Option<String>, Option<GameScoreMissing>)>,
}

impl<'a> HourNames<'a> {
    fn read(hour: ScoreHour<'_>, countries: &[CountryRow], localisation: &'a Localisation) -> Self {
        let names = CountryNames::read(&hour.frame, localisation);
        // This scan does not read ReservedCountryNaming, so slot zero's refusal
        // names its hour: names that can reach it are read every hour.
        let hourly = countries.iter().any(|row| {
            row.canonical_slot == 0
                || names
                    .operands(row.canonical_slot)
                    .and_then(|input| input.as_ref().ok())
                    .is_some_and(|input| input.overlord == Some(0))
        });
        let read = if hourly {
            Vec::new()
        } else {
            country_names::TABLES
                .iter()
                .map(|&table| match (hour.epochs, hour.view(table)) {
                    (None, Some(view)) => Incomplete::View(view.clone()),
                    (epochs, _) => {
                        Incomplete::Epoch(epochs.and_then(|epochs| epochs.get(table)))
                    }
                })
                .collect()
        };
        Self {
            hourly,
            read,
            names,
            resolved: HashMap::new(),
        }
    }

    /// Whether `hour` agrees on every operand these names were read from.
    fn agrees(&self, hour: ScoreHour<'_>) -> bool {
        !self.hourly
            && self.read.len() == country_names::TABLES.len()
            && country_names::TABLES
                .iter()
                .zip(&self.read)
                .all(|(&table, read)| match (read, hour.epochs) {
                    (Incomplete::View(view), None) => hour
                        .view(table)
                        .is_some_and(|now| same_name_rows(table, view, now)),
                    (Incomplete::Epoch(epoch), Some(epochs)) => epochs.same(table, *epoch),
                    _ => false,
                })
    }

    fn name(&mut self, slot: u32) -> (Option<String>, Option<GameScoreMissing>) {
        let names = &self.names;
        self.resolved
            .entry(slot)
            .or_insert_with(|| name_at(names, slot))
            .clone()
    }
}

fn name_at(names: &CountryNames<'_>, slot: u32) -> (Option<String>, Option<GameScoreMissing>) {
    let reason = match names
        .name(slot)
        .and_then(|name| catalogue::plain_text(&name))
    {
        Ok(name) => return (Some(name), None),
        Err(reason) => reason,
    };
    // Classify only the operand failure reached by the shared resolver. An
    // unused overlord's missing operands cannot withhold a resolved own name.
    let mut required = Some(slot);
    let mut visited = BTreeSet::new();
    while let Some(slot) = required {
        if !visited.insert(slot) {
            break;
        } // The shared resolver names cycles.
        match names.operands(slot) {
            Some(Ok(operands)) => required = operands.overlord,
            Some(Err(operand_reason)) => {
                if *operand_reason == reason {
                    return (None, Some(recording_missing(reason)));
                }
                break;
            }
            None => {
                if reason == format!("country slot {slot} has no naming operands") {
                    return (None, Some(recording_missing(reason)));
                }
                break;
            }
        }
    }
    (
        None,
        Some(GameScoreMissing {
            kind: GameScoreMissingKind::Content,
            reason,
        }),
    )
}

fn note_missing(
    missing: Option<&GameScoreMissing>,
    warnings: &mut BTreeMap<(GameScoreMissingKind, String), u64>,
) {
    if let Some(missing) = missing {
        *warnings
            .entry((missing.kind.clone(), missing.reason.clone()))
            .or_default() += 1;
    }
}

pub fn check_content(catalogue: &catalogue::Catalogue) -> Vec<dashboard_core::findings::Finding> {
    use dashboard_core::findings::{Finding, Kind};
    let mut warnings = BTreeMap::new();
    score_hints(Some(&catalogue.localisation), &mut warnings);
    let mut findings: Vec<_> = warnings
        .into_keys()
        .map(|(_, reason)| Finding::new(Kind::Refused, "game score hints", reason))
        .collect();
    let defines =
        installed_content_core::defines::read(&catalogue.content.layers).map_err(|error| error.to_string());
    for amount in content_multipliers(defines.as_ref().map_err(String::as_str)).amounts() {
        if let Some(missing) = &amount.missing {
            findings.push(Finding::new(
                Kind::Refused,
                "game score content multipliers",
                &missing.reason,
            ));
        }
    }
    findings
}

fn score_labels() -> BTreeMap<String, String> {
    [
        ("country", "Country"),
        ("total", "Total score"),
        ("army", "Army"),
        ("navy", "Navy"),
        ("air", "Air"),
        ("industry", "Industry"),
        ("land", "Land"),
        ("faction", "Faction"),
        ("multiplier", "Score multiplier"),
    ]
    .into_iter()
    .map(|(field, label)| (field.to_owned(), label.to_owned()))
    .collect()
}

fn score_hints(
    localisation: Option<&Localisation>,
    warnings: &mut BTreeMap<(GameScoreMissingKind, String), u64>,
) -> BTreeMap<String, String> {
    // The end-screen sort controls take their text from the recorded menubar
    // GUI/localisation. Keep translated phrases intact rather than stripping
    // an English Sort by prefix.
    let keys = [
        ("country", "ENDGAME_SORT_NAME"),
        ("total", "ENDGAME_SORT_SCORE"),
        ("army", "ENDGAME_SORT_ARMY"),
        ("navy", "ENDGAME_SORT_NAVY"),
        ("air", "ENDGAME_SORT_AIR"),
        ("industry", "ENDGAME_SORT_INDUSTRY"),
        ("land", "ENDGAME_SORT_PROVINCE"),
    ];
    keys.into_iter()
        .filter_map(|(field, key)| {
            let resolved = localisation
                .ok_or_else(|| format!("localisation {key}: recorded content unavailable"))
                .and_then(|localisation| {
                    if localisation.get(key).is_none() {
                        return Err(format!("localisation {key}: key is not defined"));
                    }
                    catalogue::localised_with(localisation, key, &[], catalogue::View::Scripted)
                });
            let label = match resolved {
                Ok(label) => label,
                Err(reason) => {
                    *warnings
                        .entry((GameScoreMissingKind::Content, reason))
                        .or_default() += 1;
                    return None;
                }
            };
            Some((field.to_owned(), label))
        })
        .collect()
}

#[cfg(test)]
mod columnar_tests;
#[cfg(test)]
mod tests;
