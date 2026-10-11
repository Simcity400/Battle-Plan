//! End-screen country scores at each acquisition read, never cumulative totals.
//! The end screen calls current getters rather than hourly Army/Navy/Air caches.
//! Industry and Land are separate from Total.

use super::{
    BranchParams, LoadContext, MAX_PRODUCT_REPLY_BYTES, Product,
    country_history::{SlotRows, unique_by_slot},
    country_styles::HourStyles,
    fixed5_runs::{Fixed5State, NativeFixed5Run, push_fixed5},
    table_missing_reason,
    war_casualties::same_name_rows,
};
use crate::{
    Source, catalogue, content, faction_names,
    metadata::{self, TaggedCountry, tagged_countries},
};
use installed_content::country_names::{self, CountryNames};
use installed_content::{faction_templates::Catalog, localisation::Localisation};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, SharedView, ViewIdentity},
    rows::{
        AirWingGameScoreStatsRow, CountryFactoriesRow, CountryGameScoreInputRow, CountryRow,
        FactionMemberRow, GameScoreAirSite, GameScoreMultiplierRow, TableId, tables,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct GameScore;
const Q: i64 = 100_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum GameScoreMissingKind {
    Recording,
    Content,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub struct GameScoreMissing {
    pub kind: GameScoreMissingKind,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
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

    fn extend_equal(&mut self, hour: i32) {
        for runs in [
            &mut self.army,
            &mut self.navy,
            &mut self.air,
            &mut self.industry,
            &mut self.land,
            &mut self.total,
        ] {
            if let Some(last) = runs.last_mut()
                && last.last_hour.checked_add(1) == Some(hour)
            {
                last.last_hour = hour;
            }
        }
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
pub struct GameScoreFaction {
    pub id: String,
    pub name: Option<String>,
    pub name_missing: Option<GameScoreMissing>,
    pub ordinal: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
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

#[derive(Clone, Debug, Serialize, TS)]
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
    frame: &DecodedFrame,
    old: bool,
    old_multipliers: &mut impl FnMut() -> GameScoreValues,
) -> GameScoreValues {
    if old
        || matches!(&*frame.tables[TableId::GameScoreMultipliers.index()],
        replay::frame::TableView::Unavailable { reason } if reason == replay::format::GAME_SCORE_DISABLED_REASON)
    {
        return old_multipliers();
    }
    let observed = frame.complete::<tables::GameScoreMultipliers>();
    let absent = table_missing_reason(frame, TableId::GameScoreMultipliers).unwrap_or_else(|| {
        "GameScoreMultipliers does not contain one singleton at this read".into()
    });
    let row = observed
        .ok()
        .filter(|rows| rows.len() == 1)
        .and_then(|rows| rows.first());
    multipliers(row, &absent)
}

// Defines::fixed5 owns Lua double -> f32 and separate multiply, add, subtract
// and truncating integer conversion. Old recordings have no runtime define read.
fn content_multipliers(defines: Result<&installed_content::defines::Defines, &str>) -> GameScoreValues {
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
    let length =
        crate::transport::json_len(&run.value, MAX_PRODUCT_REPLY_BYTES).ok_or_else(oversized)?;
    *budget = budget.checked_add(length).ok_or_else(oversized)?;
    if *budget > MAX_PRODUCT_REPLY_BYTES {
        return Err(oversized());
    }
    runs.push(run);
    Ok(())
}

fn oversized() -> io::Error {
    crate::invalid("game_score reply exceeds MAX_PRODUCT_REPLY_BYTES")
}

fn air_score(
    frame: &DecodedFrame,
    row: &CountryGameScoreInputRow,
    slot: u32,
    multiplier: &GameScoreAmount,
    // Complete tables, whose rows ascend by their unique keys.
    wings: Option<&[AirWingGameScoreStatsRow]>,
    registrations: Option<&[replay::rows::AirBaseRegistrationRow]>,
) -> GameScoreAmount {
    match row.air_state {
        CountryGameScoreInputRow::AIR_ABSENT => return GameScoreAmount::known(0, true),
        CountryGameScoreInputRow::AIR_PRESENT => {}
        _ => {
            return GameScoreAmount::recording(
                bytes_reason(&row.air_error, "strategic-air entry unreadable"),
                true,
            );
        }
    }
    let sum = (|| -> Result<i64, String> {
        let mut sum = 0_i64;
        for site in &row.first_sites {
            match site.access {
                GameScoreAirSite::ACCESS_FALSE => continue, // Access is false regardless of capacity.
                GameScoreAirSite::ACCESS_TRUE => {}
                _ => return Err(bytes_reason(&site.error, "air-base access unreadable")),
            }
            let registrations = registrations.ok_or_else(|| {
                table_missing_reason(frame, TableId::AirBaseRegistrations)
                    .unwrap_or_else(|| "AirBaseRegistrations join unavailable".into())
            })?;
            let registration = registrations
                .binary_search_by_key(&(site.base, slot), |entry| (entry.base, entry.slot))
                .map(|index| &registrations[index])
                .map_err(|_| {
                    format!(
                        "accessible air base {} has no AirBaseRegistrations row for country slot {slot}",
                        site.base
                    )
                })?;
            for id in &registration.wings {
                let wings = wings.ok_or_else(|| {
                    table_missing_reason(frame, TableId::AirWingGameScoreStats)
                        .unwrap_or_else(|| "AirWingGameScoreStats join unavailable".into())
                })?;
                let wing = wings
                    .binary_search_by_key(id, |wing| wing.id)
                    .map(|index| &wings[index])
                    .map_err(|_| {
                        format!("air wing {id} has no recorded mission-1 score-stat companion")
                    })?;
                let planes = wing.planes.ok_or_else(|| {
                    bytes_reason(&wing.planes_error, "air-wing plane count unreadable")
                })?;
                let stat = wing.stat_raw.ok_or_else(|| {
                    bytes_reason(&wing.stat_error, "air-wing mission-1 stat 0x33 unreadable")
                })?;
                sum = sum.wrapping_add(air_contribution(planes, stat));
            }
        }
        Ok(sum)
    })();
    match (sum, multiplier.value()) {
        (Ok(sum), Some(multiplier)) => {
            GameScoreAmount::known(sum.wrapping_mul(multiplier) / Q, true)
        }
        (Err(reason), _) => GameScoreAmount::recording(reason, true),
        (_, None) => multiplier.clone(),
    }
}

struct ScoreContext<'a> {
    frame: &'a DecodedFrame,
    multipliers: &'a GameScoreValues,
    input_refusal: Option<&'a str>,
    wings: Option<&'a [AirWingGameScoreStatsRow]>,
    registrations: Option<&'a [replay::rows::AirBaseRegistrationRow]>,
}

impl ScoreContext<'_> {
    fn scores(
        &self,
        input: Option<&CountryGameScoreInputRow>,
        slot: u32,
        factories: Option<&CountryFactoriesRow>,
    ) -> GameScoreValues {
        let Self {
            frame,
            multipliers,
            input_refusal,
            wings,
            registrations,
        } = *self;
        let input_missing = table_missing_reason(frame, TableId::CountryGameScoreInputs);
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
            result.air = air_score(frame, row, slot, &multipliers.air, wings, registrations);
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
            (None, _) => GameScoreAmount::recording(table_missing_reason(frame, TableId::CountryFactories)
                .unwrap_or_else(|| "CountryFactories is complete but this country's row is missing or ambiguous".into()), false),
            (_, None) => multipliers.industry.clone(),
        };
        result
    }
}

type FactionName = (Option<String>, Option<GameScoreMissing>);

struct FactionLexical {
    tokens: faction_names::LexicalNames,
    missing: Option<String>,
}

impl FactionLexical {
    fn new(frame: &DecodedFrame) -> Self {
        Self {
            tokens: faction_names::LexicalNames::new(
                frame.tables[TableId::LexicalTokens.index()].clone(),
            ),
            missing: table_missing_reason(frame, TableId::LexicalTokens),
        }
    }
}

fn faction_names_at(
    frame: &DecodedFrame,
    localisation: Option<&Localisation>,
    fallback: &Localisation,
    recorded_language: &str,
    templates: Result<&Catalog, String>,
    lexical: &FactionLexical,
) -> Option<HashMap<u64, FactionName>> {
    let rows = frame.complete::<tables::Factions>().ok()?;
    Some(
        rows.iter()
            .map(|row| {
                // Faction-name literals bypass both the template and lexical dictionary;
                // empty literals use the shared bound-name rule.
                let template = match (row.template_token, &lexical.missing) {
                    (Some(_), Some(reason)) => Err(reason.clone()),
                    _ => faction_names::template_spelling(frame, row.template_token),
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
                            None => lexical.tokens.token_of(name),
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
    )
}

fn faction(
    members: Option<&SlotRows<'_, FactionMemberRow>>,
    names: Option<&HashMap<u64, FactionName>>,
    slot: u32,
) -> (Option<GameScoreFaction>, Option<GameScoreMissing>) {
    let Some(mapped) = members else {
        return (
            None,
            Some(recording_missing("FactionMembers is not complete")),
        );
    };
    let member = match mapped.rows.get(&slot) {
        None if mapped.unresolved_rows == 0 => return (None, None),
        Some(Some(member)) => *member,
        _ => {
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
        ]
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let replay = source.replay();
        let localisation = content::name_localisation(source, context.content.as_ref());
        let mut old_multipliers = None;
        let mut read_old_multipliers = || {
            old_multipliers
                .get_or_insert_with(|| {
                    let defines = context
                        .content
                        .as_ref()
                        .ok_or_else(|| {
                            "Choose the recorded content folders to read score defines".to_owned()
                        })
                        .and_then(|options| {
                            catalogue::catalogue(source, options).map_err(|error| error.to_string())
                        })
                        .and_then(|cat| {
                            installed_content::defines::read(&cat.content.layers)
                                .map_err(|error| error.to_string())
                        });
                    content_multipliers(defines.as_ref().map_err(String::as_str))
                })
                .clone()
        };
        let mut naming_warnings = Vec::new();
        let templates = context
            .content
            .as_ref()
            .ok_or_else(|| {
                "Faction naming content is unavailable until the game folders are chosen".to_owned()
            })
            .and_then(|options| {
                content::check_replay(&replay, options.allow_unverified, &mut naming_warnings)
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
        let mut reply = scan(
            &replay,
            params,
            localisation.as_deref().ok(),
            templates.as_deref().map_err(Clone::clone),
            viewer_language.as_deref(),
            &mut read_old_multipliers,
            cancelled,
        )?;
        reply.warnings.extend(naming_warnings);
        if let Ok(templates) = templates {
            reply.warnings.extend(templates.warnings.iter().cloned());
        }
        if crate::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
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
        return Err(crate::invalid("game_score branch is not recorded"));
    }
    let selected = [
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
    let mut previous: Option<(i32, Vec<ViewIdentity>)> = None;
    let mut previous_warnings = BTreeMap::<(GameScoreMissingKind, String), u64>::new();
    let language = &replay.identity.language;
    let recorded_language = language.strip_prefix("l_").unwrap_or(language);
    let mut empty_localisation = Localisation::default();
    empty_localisation.language = viewer_language.unwrap_or(recorded_language).to_owned();
    let mut lexical: Option<FactionLexical> = None;
    let mut names: Option<HourNames<'_>> = None;
    let input_refusal = replay.format.table_refusal(TableId::CountryGameScoreInputs);
    crate::blocks::scan_blocks(
        replay,
        replay
            .index
            .iter()
            .filter(|block| block.branch == params.branch()),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                metadata::check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
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
                        return Err(crate::invalid("game_score request cancelled"));
                    }
                    let decoded = match decoder.advance(&frame) {
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
                    // Table lifetimes, rather than a capture failure's text, establish
                    // whether runtime operands were part of this recording's contract.
                    let old = !TableId::CountryGameScoreInputs
                        .last_change()
                        .present(replay.format.version());
                    let (Ok(country_rows), Ok(links)) = (
                        decoded.complete::<tables::Countries>(),
                        decoded.complete::<tables::CountryLinks>(),
                    ) else {
                        let multipliers = multipliers_at(decoded, old, old_multipliers);
                        for amount in multipliers.amounts() {
                            note_missing(amount.missing.as_ref(), &mut warnings);
                        }
                        reply.multipliers.push(
                            frame.hour,
                            &multipliers,
                            false,
                            &mut reasons,
                            &mut budget,
                        )?;
                        *warnings
                        .entry((
                            GameScoreMissingKind::Recording,
                            "Countries or CountryLinks is incomplete; country identities are unknown"
                                .into(),
                        ))
                        .or_default() += 1;
                        previous = None;
                        continue;
                    };
                    if previous.as_ref().is_some_and(|(hour, views)| {
                        hour.checked_add(1) == Some(frame.hour)
                            && views
                                .iter()
                                .zip(selected)
                                .all(|(before, table)| before.is(&decoded.tables[table.index()]))
                    }) {
                        for series in countries.values_mut() {
                            series.scores.extend_equal(frame.hour);
                            if let Some(last) = series.identity.last_mut()
                                && last.last_hour.checked_add(1) == Some(frame.hour)
                            {
                                last.last_hour = frame.hour;
                            }
                        }
                        reply.multipliers.extend_equal(frame.hour);
                        for (key, count) in &previous_warnings {
                            *warnings.entry(key.clone()).or_default() += count;
                        }
                        if let Some(previous) = previous.as_mut() {
                            previous.0 = frame.hour;
                        }
                        continue;
                    }
                    let mut frame_warnings = BTreeMap::new();
                    let multipliers = multipliers_at(decoded, old, old_multipliers);
                    reply.multipliers.push(
                        frame.hour,
                        &multipliers,
                        false,
                        &mut reasons,
                        &mut budget,
                    )?;
                    for amount in multipliers.amounts() {
                        note_missing(amount.missing.as_ref(), &mut frame_warnings);
                    }
                    let input_rows = decoded.complete::<tables::CountryGameScoreInputs>().ok();
                    let inputs =
                        unique_by_slot(input_rows.into_iter().flatten(), links, |row| row.country);
                    for row in input_rows.into_iter().flatten() {
                        if metadata::link_slot(links, row.country).is_none() {
                            *frame_warnings.entry((GameScoreMissingKind::Recording, format!(
                            "CountryGameScoreInputs country raw reference {} has no CountryLinks identity at this read", row.country))).or_default() += 1;
                        }
                    }
                    let factory_rows = decoded.complete::<tables::CountryFactories>().ok();
                    let factories =
                        unique_by_slot(factory_rows.into_iter().flatten(), links, |row| {
                            row.country
                        });
                    let wings = decoded.complete::<tables::AirWingGameScoreStats>().ok();
                    let registrations = decoded.complete::<tables::AirBaseRegistrations>().ok();
                    let members = decoded
                        .complete::<tables::FactionMembers>()
                        .ok()
                        .map(|rows| unique_by_slot(rows, links, |row| row.country));
                    if lexical.as_ref().is_none_or(|previous| {
                        !previous
                            .tokens
                            .matches(&decoded.tables[TableId::LexicalTokens.index()])
                    }) {
                        lexical = Some(FactionLexical::new(decoded));
                    }
                    let factions = lexical.as_ref().and_then(|lexical| {
                        faction_names_at(
                            decoded,
                            localisation,
                            &empty_localisation,
                            recorded_language,
                            templates.as_ref().copied().map_err(Clone::clone),
                            lexical,
                        )
                    });
                    let styles = HourStyles::read(decoded, None)?;
                    if names.as_ref().is_none_or(|read| !read.agrees(decoded)) {
                        names = Some(HourNames::read(
                            decoded,
                            country_rows,
                            localisation.unwrap_or(&empty_localisation),
                        ));
                    }
                    let names = names.as_mut().expect("names agree with this hour");
                    for (tag, country) in tagged_countries(country_rows, links) {
                        let tag = match std::str::from_utf8(tag) {
                            Ok(tag) => tag.to_owned(),
                            Err(_) => {
                                *frame_warnings
                                .entry((
                                    GameScoreMissingKind::Recording,
                                    format!(
                                        "game_score country tag bytes {tag:02x?} refused: not UTF-8"
                                    ),
                                ))
                                .or_default() += 1;
                                continue;
                            }
                        };
                        let (values, identity) = match country {
                            TaggedCountry::Found(country, slot) => {
                                let (faction, faction_missing) =
                                    faction(members.as_ref(), factions.as_ref(), slot);
                                let style = styles.get(&tag);
                                let (name, name_missing) = names.name(slot);
                                let color = style.and_then(|style| style.color);
                                let color_missing = color.is_none().then(|| recording_missing(
                                table_missing_reason(decoded, TableId::CountryAppearance)
                                    .unwrap_or_else(|| "CountryAppearance has no unambiguous colour for this country".into())));
                                let identity = GameScoreIdentity {
                                    canonical_slot: Some(slot),
                                    native_country: Some(country.id),
                                    name,
                                    name_missing,
                                    color,
                                    color_missing,
                                    faction,
                                    faction_missing,
                                };
                                let context = ScoreContext {
                                    frame: decoded,
                                    multipliers: &multipliers,
                                    input_refusal: input_refusal.as_deref(),
                                    wings,
                                    registrations,
                                };
                                (
                                    context.scores(
                                        inputs.rows.get(&slot).copied().flatten(),
                                        slot,
                                        factories.rows.get(&slot).copied().flatten(),
                                    ),
                                    identity,
                                )
                            }
                            _ => {
                                let missing = recording_missing(
                                    "Country identity is unresolved by CountryLinks",
                                );
                                (
                                    unknown_values(&missing.reason),
                                    GameScoreIdentity {
                                        canonical_slot: None,
                                        native_country: None,
                                        name: None,
                                        name_missing: Some(missing.clone()),
                                        color: None,
                                        color_missing: Some(missing.clone()),
                                        faction: None,
                                        faction_missing: Some(missing),
                                    },
                                )
                            }
                        };
                        for amount in values.amounts() {
                            note_missing(amount.missing.as_ref(), &mut frame_warnings);
                        }
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
                        let series =
                            countries
                                .entry(tag.clone())
                                .or_insert_with(|| GameScoreCountry {
                                    tag,
                                    identity: Vec::new(),
                                    scores: GameScoreSeries::default(),
                                });
                        // A changed native identity starts a new score stretch even if values match.
                        let same_identity = series.identity.last().is_some_and(|run| {
                            run.last_hour.checked_add(1) == Some(frame.hour)
                                && run.value.canonical_slot == identity.canonical_slot
                                && run.value.native_country == identity.native_country
                        });
                        series.scores.push(
                            frame.hour,
                            &values,
                            !same_identity,
                            &mut reasons,
                            &mut budget,
                        )?;
                        push_run(
                            &mut series.identity,
                            frame.hour,
                            identity,
                            false,
                            &mut budget,
                        )?;
                    }
                    for (key, count) in &frame_warnings {
                        *warnings.entry(key.clone()).or_default() += count;
                    }
                    previous_warnings = frame_warnings;
                    // Holding the views would copy each one the decoder next updates.
                    previous = Some((
                        frame.hour,
                        selected
                            .iter()
                            .map(|table| decoded.tables[table.index()].identity())
                            .collect(),
                    ));
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
    if crate::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(oversized());
    }
    Ok(reply)
}

/// Names read at the last hour whose naming operands changed, each resolved
/// once. Holding that hour's views copies each at most once, at its next update.
struct HourNames<'a> {
    /// By `country_names::TABLES`; none when names are read every hour.
    views: Vec<SharedView>,
    names: CountryNames<'a>,
    resolved: HashMap<u32, (Option<String>, Option<GameScoreMissing>)>,
}

impl<'a> HourNames<'a> {
    fn read(
        frame: &DecodedFrame,
        countries: &[CountryRow],
        localisation: &'a Localisation,
    ) -> Self {
        let names = CountryNames::read(frame, localisation);
        // This scan does not read ReservedCountryNaming, so slot zero's refusal
        // names its hour: names that can reach it are read every hour.
        let hourly = countries.iter().any(|row| {
            row.canonical_slot == 0
                || names
                    .operands(row.canonical_slot)
                    .and_then(|input| input.as_ref().ok())
                    .is_some_and(|input| input.overlord == Some(0))
        });
        let views = if hourly {
            Vec::new()
        } else {
            country_names::TABLES
                .iter()
                .map(|table| frame.tables[table.index()].clone())
                .collect()
        };
        Self {
            views,
            names,
            resolved: HashMap::new(),
        }
    }

    /// Whether `frame` agrees on every operand these names were read from.
    fn agrees(&self, frame: &DecodedFrame) -> bool {
        self.views.len() == country_names::TABLES.len()
            && country_names::TABLES
                .iter()
                .zip(&self.views)
                .all(|(table, view)| same_name_rows(*table, view, &frame.tables[table.index()]))
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

pub(crate) fn check_content(
    catalogue: &catalogue::Catalogue,
) -> Vec<crate::content_check::Finding> {
    use crate::content_check::{Finding, Kind};
    let mut warnings = BTreeMap::new();
    score_hints(Some(&catalogue.localisation), &mut warnings);
    let mut findings: Vec<_> = warnings
        .into_keys()
        .map(|(_, reason)| Finding::new(Kind::Refused, "game score hints", reason))
        .collect();
    let defines =
        installed_content::defines::read(&catalogue.content.layers).map_err(|error| error.to_string());
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
mod tests;
