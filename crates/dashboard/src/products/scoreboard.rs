//! An exact table of shared per-country measures. There is no overall score.
//! Factories reuse the game's whole-count parts and count modes (140E57F10,
//! 14187F760); political/command balances reuse country_measure (stored fixed5
//! CPoliticalStatus+0xE0, AddPoliticalPower 140B98130; CCountry+0x1F0,
//! AddCommandPower 1406C3320); stability and
//! war support reuse GetStability 1406EBBE0 / GetWarSupport 1406ED300 arithmetic;
//! nukes reuse GetNukes 1410828A0. Equipment totals reuse the cached production
//! scan and its exact delivered-unit helper, never another production walk.
//! Owned states use the recorded native roster count: SetOwner 1409D1710,
//! AddOwnedState 1406C52B0 (+0x478/+0x484), excluding zero-province templates.
//! This uses the shared unambiguous country-link join. Identities
//! are independent of value coverage, so unknown amounts rank last, never zero.

use super::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product,
    country_factories::{self, FactoryCount, FactoryGroupKey},
    country_groups::HourSpan,
    country_history::{self, CountryIdentityRun, RecordedHistories},
    country_measure::FixedRun,
    country_nukes, country_stability, equipment, equipment_produced, validate_tag,
};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    ranking,
    transport::json_len,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct Scoreboard;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum ScoreboardColumnKey {
    Factories,
    ArmySize,
    Territory,
    Casualties,
    Planes,
    Ships,
    FactoriesBuilt,
    FactoriesTopbar,
    EquipmentProduced,
    PoliticalPower,
    CommandPower,
    Stability,
    WarSupport,
    Nukes,
}

impl ScoreboardColumnKey {
    pub const ALL: [Self; 14] = [
        Self::Factories,
        Self::ArmySize,
        Self::Territory,
        Self::Casualties,
        Self::Planes,
        Self::Ships,
        Self::FactoriesBuilt,
        Self::FactoriesTopbar,
        Self::EquipmentProduced,
        Self::PoliticalPower,
        Self::CommandPower,
        Self::Stability,
        Self::WarSupport,
        Self::Nukes,
    ];

    fn military_adapter(self) -> Option<MilitaryAdapter> {
        match self {
            Self::ArmySize => Some(army_size),
            Self::Planes => Some(planes),
            Self::Ships => Some(ships),
            Self::Casualties => Some(casualties),
            _ => None,
        }
    }

    fn unavailable_reason(self) -> Option<&'static str> {
        self.military_adapter()
            .and_then(|adapter| adapter(None, 0, &[], &AtomicBool::new(false)).err())
    }

    fn factory_count(self) -> Option<FactoryCount> {
        match self {
            Self::Factories => Some(FactoryCount::Free),
            Self::FactoriesBuilt => Some(FactoryCount::Built),
            Self::FactoriesTopbar => Some(FactoryCount::TopBar),
            _ => None,
        }
    }
}

// One swap point per Military measure. None asks only about availability; Some
// supplies the source for its shared per-country, per-hour measure. Registry,
// validation and values all call the same adapter, so wiring it changes one body.
type MilitaryAdapter =
    fn(Option<&Source>, u64, &[String], &AtomicBool) -> Result<ColumnValues, &'static str>;
fn army_size(
    _source: Option<&Source>,
    _branch: u64,
    _tags: &[String],
    _cancelled: &AtomicBool,
) -> Result<ColumnValues, &'static str> {
    Err(
        "Not available yet: Army size comes with the Army size widget's shared measure; it is not on main yet.",
    )
}
fn planes(
    _source: Option<&Source>,
    _branch: u64,
    _tags: &[String],
    _cancelled: &AtomicBool,
) -> Result<ColumnValues, &'static str> {
    Err(
        "Not available yet: Number of planes comes with the Planes widget's shared measure; it is not on main yet.",
    )
}
fn ships(
    _source: Option<&Source>,
    _branch: u64,
    _tags: &[String],
    _cancelled: &AtomicBool,
) -> Result<ColumnValues, &'static str> {
    Err(
        "Not available yet: Number of ships comes with the Ships widget's shared measure; it is not on main yet.",
    )
}
fn casualties(
    _source: Option<&Source>,
    _branch: u64,
    _tags: &[String],
    _cancelled: &AtomicBool,
) -> Result<ColumnValues, &'static str> {
    Err(
        "Not available yet: Casualties comes with the Casualties widget's shared measure; it is not on main yet.",
    )
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScoreboardTop {
    pub count: u32,
    pub sort: ScoreboardColumnKey,
    pub descending: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScoreboardParams {
    pub branch_id: String,
    pub columns: Vec<ScoreboardColumnKey>,
    pub tags: Vec<String>,
    pub top: Option<ScoreboardTop>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardColumn {
    pub key: ScoreboardColumnKey,
    pub label: String,
    pub unit: String,
    pub decimals: u8,
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Exact signed integer in units of 10^-decimals. None is unknown.
    pub amount: Option<String>,
    pub starts_segment: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardReadRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub read_hour: i32,
    /// Every hour is a supplying read; otherwise read_hour supplies this span.
    pub hourly: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardColumnSeries {
    pub key: ScoreboardColumnKey,
    pub runs: Vec<ScoreboardRun>,
    pub read_runs: Vec<ScoreboardReadRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardSeries {
    pub tag: String,
    pub segments: Vec<HourSpan>,
    pub identities: Vec<CountryIdentityRun>,
    pub membership: Option<Vec<HourSpan>>,
    pub columns: Vec<ScoreboardColumnSeries>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardReply {
    pub branch_id: String,
    pub default_columns: Vec<ScoreboardColumnKey>,
    pub default_sort: ScoreboardColumnKey,
    /// The one complete column registry, also returned for settings-only reads.
    pub columns: Vec<ScoreboardColumn>,
    pub series: Vec<ScoreboardSeries>,
    pub warnings: Vec<String>,
}

/// UI words for counts with no matching game label; native GUI label bindings
/// are resolved from the recorded content by registry(). No vanilla text is
/// used as a content lookup key.
/// S: The specific in-game display binding for the proven owned-state roster
/// count is unverified; "States owned" is a plain UI description.
struct ColumnDescriptor {
    label: &'static str,
    unit: &'static str,
    decimals: u8,
}

fn descriptor(key: ScoreboardColumnKey) -> ColumnDescriptor {
    use ScoreboardColumnKey::*;
    match key {
        Factories => ColumnDescriptor {
            label: "Factories",
            unit: "factories",
            decimals: 0,
        },
        FactoriesBuilt => ColumnDescriptor {
            label: "Factories built",
            unit: "factories",
            decimals: 0,
        },
        FactoriesTopbar => ColumnDescriptor {
            label: "Factories in top bar",
            unit: "factories",
            decimals: 0,
        },
        Territory => ColumnDescriptor {
            label: "States owned",
            unit: "states",
            decimals: 0,
        },
        ArmySize => ColumnDescriptor {
            label: "Army size",
            unit: "divisions",
            decimals: 0,
        },
        Casualties => ColumnDescriptor {
            label: "Casualties",
            unit: "people",
            decimals: 0,
        },
        Planes => ColumnDescriptor {
            label: "Number of planes",
            unit: "planes",
            decimals: 0,
        },
        Ships => ColumnDescriptor {
            label: "Number of ships",
            unit: "ships",
            decimals: 0,
        },
        EquipmentProduced => ColumnDescriptor {
            label: "Equipment produced",
            unit: "units",
            decimals: 0,
        },
        PoliticalPower => ColumnDescriptor {
            label: "Political power",
            unit: "points",
            decimals: 5,
        },
        CommandPower => ColumnDescriptor {
            label: "Command power",
            unit: "points",
            decimals: 5,
        },
        Stability => ColumnDescriptor {
            label: "Stability",
            unit: "percent",
            decimals: 5,
        },
        WarSupport => ColumnDescriptor {
            label: "War support",
            unit: "percent",
            decimals: 5,
        },
        Nukes => ColumnDescriptor {
            label: "Nukes",
            unit: "nukes",
            decimals: 0,
        },
    }
}

fn registry(catalogue: Option<&Catalogue>) -> Vec<ScoreboardColumn> {
    ScoreboardColumnKey::ALL
        .into_iter()
        .map(|key| {
            let description = descriptor(key);
            let reason = key.unavailable_reason().map(str::to_owned);
            // P: Political tooltip 140B9A1D0 binds POLITICAL_POWER (142939A20).
            // GetStability 1406EBBE0 / GetWarSupport 1406ED300 and the top-bar
            // tooltip 1418845C0 bind STABILITY (1427D1B08) / WAR_SUPPORT (1427D1C58).
            // Its command-power heading binds COMMAND_POWER at 141886ED7.
            // These keys are executable facts; every translation is recorded content.
            let native_key = match key {
                ScoreboardColumnKey::PoliticalPower => Some("POLITICAL_POWER"),
                ScoreboardColumnKey::CommandPower => Some("COMMAND_POWER"),
                ScoreboardColumnKey::Stability => Some("STABILITY"),
                ScoreboardColumnKey::WarSupport => Some("WAR_SUPPORT"),
                _ => None,
            };
            let label = native_key
                .and_then(|key| {
                    catalogue.and_then(|cat| cat.localised(key, catalogue::View::Plain))
                })
                .unwrap_or_else(|| description.label.into());
            ScoreboardColumn {
                key,
                label,
                unit: description.unit.into(),
                decimals: description.decimals,
                available: reason.is_none(),
                reason,
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ValueRun {
    first_hour: i32,
    last_hour: i32,
    amount: Option<i64>,
    starts_segment: bool,
}

struct ColumnData {
    key: ScoreboardColumnKey,
    runs: Vec<ValueRun>,
    read_runs: Vec<ScoreboardReadRun>,
}

impl ColumnData {
    fn reply(self) -> ScoreboardColumnSeries {
        ScoreboardColumnSeries {
            key: self.key,
            runs: self
                .runs
                .into_iter()
                .map(|run| ScoreboardRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    amount: run.amount.map(|amount| amount.to_string()),
                    starts_segment: run.starts_segment,
                })
                .collect(),
            read_runs: self.read_runs,
        }
    }
}

fn numeric_fixed_runs(runs: &[FixedRun]) -> io::Result<Vec<ValueRun>> {
    runs.iter()
        .map(|run| {
            Ok(ValueRun {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
                amount: Some(run.value_raw.parse().map_err(|_| {
                    io::Error::other("The shared fixed5 measure is not a signed native integer")
                })?),
                starts_segment: run.starts_segment,
            })
        })
        .collect()
}

/// Merge sorted value coverage and held-read intervals in one forward walk.
/// The read cursor stays on a carried interval spanning several value runs.
fn stability_reads(
    runs: &[ValueRun],
    carried: &[country_stability::ReadRun],
) -> Vec<ScoreboardReadRun> {
    let mut out = Vec::new();
    let mut cursor = 0;
    for run in runs {
        let mut first = i64::from(run.first_hour);
        while cursor < carried.len() && carried[cursor].last_hour < run.first_hour {
            cursor += 1;
        }
        while first <= i64::from(run.last_hour) && cursor < carried.len() {
            let held = &carried[cursor];
            if held.first_hour > run.last_hour {
                break;
            }
            let start = i64::from(held.first_hour).max(first);
            if first < start {
                out.push(ScoreboardReadRun {
                    first_hour: first as i32,
                    last_hour: start as i32 - 1,
                    read_hour: first as i32,
                    hourly: true,
                });
            }
            let end = held.last_hour.min(run.last_hour);
            out.push(ScoreboardReadRun {
                first_hour: start as i32,
                last_hour: end,
                read_hour: held.read_hour,
                hourly: false,
            });
            first = i64::from(end) + 1;
            if end == held.last_hour {
                cursor += 1;
            }
        }
        if first <= i64::from(run.last_hour) {
            out.push(ScoreboardReadRun {
                first_hour: first as i32,
                last_hour: run.last_hour,
                read_hour: first as i32,
                hourly: true,
            });
        }
    }
    out
}

fn hourly_reads(runs: &[ValueRun]) -> Vec<ScoreboardReadRun> {
    let mut out: Vec<ScoreboardReadRun> = Vec::new();
    for run in runs {
        if let Some(last) = out.last_mut()
            && !run.starts_segment
            && last.last_hour.checked_add(1) == Some(run.first_hour)
        {
            last.last_hour = run.last_hour;
        } else {
            out.push(ScoreboardReadRun {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
                read_hour: run.first_hour,
                hourly: true,
            });
        }
    }
    out
}

fn read_spans(runs: &[ValueRun], reads: &[i32]) -> Vec<ScoreboardReadRun> {
    let mut out = Vec::new();
    for run in runs {
        let mut first = run.first_hour;
        let mut index = reads.partition_point(|read| *read <= first);
        while first <= run.last_hour {
            let Some(read) = index.checked_sub(1).map(|i| reads[i]) else {
                break;
            };
            let last = reads
                .get(index)
                .and_then(|read| read.checked_sub(1))
                .map_or(run.last_hour, |end| end.min(run.last_hour));
            out.push(ScoreboardReadRun {
                first_hour: first,
                last_hour: last,
                read_hour: read,
                hourly: false,
            });
            let Some(next) = last.checked_add(1) else {
                break;
            };
            first = next;
            index += 1;
        }
    }
    out
}

type ColumnValues = BTreeMap<String, ColumnData>;

fn values(
    source: &Source,
    branch: u64,
    key: ScoreboardColumnKey,
    tags: &[String],
    history: &RecordedHistories,
    cancelled: &AtomicBool,
    warnings: &mut Vec<String>,
) -> io::Result<ColumnValues> {
    let mut out = BTreeMap::new();
    if let Some(adapter) = key.military_adapter() {
        return Ok(adapter(Some(source), branch, tags, cancelled).unwrap_or_default());
    }
    if let Some(mode) = key.factory_count() {
        let scanned = country_factories::scanned_branch(source, branch, cancelled)?;
        if scanned.unreadable > 0 {
            warnings.push(format!("{} factory rows were unresolved, ambiguous or outside the game's displayed whole-count domain", scanned.unreadable));
        }
        for tag in tags {
            let runs: Vec<_> = scanned
                .series
                .get(tag)
                .into_iter()
                .flatten()
                .map(|run| ValueRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    starts_segment: run.starts_segment,
                    amount: Some(
                        FactoryGroupKey::ALL
                            .iter()
                            .map(|group| country_factories::group_count(&run.values, *group, mode))
                            .sum::<i64>(),
                    ),
                })
                .collect();
            out.insert(
                tag.clone(),
                ColumnData {
                    key,
                    runs,
                    read_runs: Vec::new(),
                },
            );
        }
        return Ok(out);
    }
    match key {
        ScoreboardColumnKey::PoliticalPower
        | ScoreboardColumnKey::CommandPower
        | ScoreboardColumnKey::Territory => {
            for tag in tags {
                let runs = history
                    .countries
                    .get(tag)
                    .map(|country| {
                        match key {
                            ScoreboardColumnKey::PoliticalPower => &country.political_power.runs,
                            ScoreboardColumnKey::CommandPower => &country.command_power.runs,
                            _ => &country.territory,
                        }
                        .iter()
                        .map(|run| ValueRun {
                            first_hour: run.first_hour,
                            last_hour: run.last_hour,
                            amount: Some(run.value),
                            starts_segment: run.starts_segment,
                        })
                        .collect()
                    })
                    .unwrap_or_default();
                out.insert(
                    tag.clone(),
                    ColumnData {
                        key,
                        runs,
                        read_runs: Vec::new(),
                    },
                );
            }
        }
        ScoreboardColumnKey::Stability | ScoreboardColumnKey::WarSupport => {
            let found = country_stability::histories(source, branch, tags, cancelled)?;
            for tag in tags {
                let Some(country) = found.get(tag) else {
                    continue;
                };
                let runs = numeric_fixed_runs(if key == ScoreboardColumnKey::Stability {
                    &country.stability
                } else {
                    &country.war_support
                })?;
                out.insert(
                    tag.clone(),
                    ColumnData {
                        key,
                        runs,
                        read_runs: Vec::new(),
                    },
                );
            }
        }
        ScoreboardColumnKey::Nukes => {
            let scanned = country_nukes::scanned_branch(source, branch, cancelled)?;
            for tag in tags {
                let runs: Vec<_> = scanned
                    .series
                    .get(tag)
                    .into_iter()
                    .flatten()
                    .map(|run| ValueRun {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                        amount: run.total.map(i64::from),
                        starts_segment: run.starts_segment,
                    })
                    .collect();
                out.insert(
                    tag.clone(),
                    ColumnData {
                        key,
                        runs,
                        read_runs: Vec::new(),
                    },
                );
            }
            for (kind, count) in &scanned.refused {
                warnings.push(format!(
                    "{count} unsupported nuclear array entries of recorded ordinal {kind}"
                ));
            }
        }
        ScoreboardColumnKey::EquipmentProduced => {
            let scanned = equipment::production::scanned(source, branch, cancelled)?;
            warnings.extend(equipment_produced::recording_warnings(&scanned));
            let found = equipment_produced::country_totals(&scanned, tags, cancelled)?;
            for (tag, entries) in found {
                let runs: Vec<_> = entries
                    .into_iter()
                    .map(|run| ValueRun {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                        amount: run.amount,
                        starts_segment: run.starts_segment,
                    })
                    .collect();
                out.insert(
                    tag,
                    ColumnData {
                        key,
                        runs,
                        read_runs: Vec::new(),
                    },
                );
            }
        }
        _ => {}
    }
    Ok(out)
}

/// Supplying dates are needed only for countries returned to the table, never
/// for the whole ranking universe. Shared scans/cache lookups do not re-read data.
fn fill_read_runs(
    source: &Source,
    branch: u64,
    key: ScoreboardColumnKey,
    tags: &[String],
    columns: &mut ColumnValues,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    match key {
        ScoreboardColumnKey::Stability | ScoreboardColumnKey::WarSupport => {
            let histories = country_stability::histories(source, branch, tags, cancelled)?;
            for tag in tags {
                if let (Some(column), Some(history)) = (columns.get_mut(tag), histories.get(tag)) {
                    column.read_runs = stability_reads(&column.runs, &history.carried);
                }
            }
        }
        ScoreboardColumnKey::EquipmentProduced => {
            let scanned = equipment::production::scanned(source, branch, cancelled)?;
            for tag in tags {
                if let Some(column) = columns.get_mut(tag) {
                    column.read_runs = read_spans(&column.runs, &scanned.reads);
                }
            }
        }
        _ => {
            for tag in tags {
                if let Some(column) = columns.get_mut(tag) {
                    column.read_runs = hourly_reads(&column.runs);
                }
            }
        }
    }
    Ok(())
}

/// Rank identity-present countries, inserting explicit unknown pieces where the
/// sort measure has no value. Values stay exact signed integers throughout.
fn ranking_runs(
    history: &RecordedHistories,
    values: &ColumnValues,
) -> BTreeMap<String, Vec<(i32, i32, Option<i128>)>> {
    history
        .countries
        .iter()
        .map(|(tag, country)| {
            let measures = values
                .get(tag)
                .map(|column| column.runs.as_slice())
                .unwrap_or(&[]);
            let mut out = Vec::new();
            let mut cursor = 0;
            for segment in country.segments.iter() {
                let mut hour = segment.first_hour;
                while hour <= segment.last_hour {
                    while measures.get(cursor).is_some_and(|run| run.last_hour < hour) {
                        cursor += 1;
                    }
                    let next = measures.get(cursor);
                    let (last, amount) = match next {
                        Some(run) if run.first_hour <= hour => (
                            run.last_hour.min(segment.last_hour),
                            run.amount.map(i128::from),
                        ),
                        Some(run) => (segment.last_hour.min(run.first_hour - 1), None),
                        None => (segment.last_hour, None),
                    };
                    out.push((hour, last, amount));
                    let Some(next) = last.checked_add(1) else {
                        break;
                    };
                    hour = next;
                }
            }
            (tag.clone(), out)
        })
        .collect()
}

fn clip_column(
    column: ColumnData,
    spans: &[HourSpan],
    cancelled: &AtomicBool,
) -> io::Result<ColumnData> {
    let mut runs = Vec::new();
    ranking::for_each_piece(
        &column.runs,
        spans,
        |run| (run.first_hour, run.last_hour),
        cancelled,
        |run, first_hour, last_hour| {
            runs.push(ValueRun {
                first_hour,
                last_hour,
                amount: run.amount,
                starts_segment: run.starts_segment && first_hour == run.first_hour,
            });
            Ok(())
        },
    )?;
    let mut read_runs = Vec::new();
    ranking::for_each_piece(
        &column.read_runs,
        spans,
        |run| (run.first_hour, run.last_hour),
        cancelled,
        |run, first_hour, last_hour| {
            read_runs.push(ScoreboardReadRun {
                first_hour,
                last_hour,
                read_hour: if run.hourly {
                    first_hour
                } else {
                    run.read_hour
                },
                hourly: run.hourly,
            });
            Ok(())
        },
    )?;
    Ok(ColumnData {
        key: column.key,
        runs,
        read_runs,
    })
}

impl Product for Scoreboard {
    const KEY: &'static str = "scoreboard";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ScoreboardParams;
    type Reply = ScoreboardReply;

    fn types() -> Vec<String> {
        vec![
            ScoreboardColumnKey::decl(),
            ScoreboardTop::decl(),
            ScoreboardParams::decl(),
            ScoreboardColumn::decl(),
            ScoreboardRun::decl(),
            ScoreboardReadRun::decl(),
            CountryIdentityRun::decl(),
            ScoreboardColumnSeries::decl(),
            ScoreboardSeries::decl(),
            ScoreboardReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid scoreboard branch")?;
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        let mut unique = BTreeSet::new();
        if params.columns.iter().any(|column| !unique.insert(*column)) {
            return Err("Invalid scoreboard columns");
        }
        if let Some(top) = &params.top
            && (!params.tags.is_empty()
                || top.count == 0
                || !params.columns.contains(&top.sort)
                || top.sort.unavailable_reason().is_some())
        {
            return Err("Invalid scoreboard top countries");
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let mut warnings = Vec::new();
        let cat = context.content.as_ref().and_then(|options| {
            match catalogue::catalogue(source, options) {
                Ok(cat) => Some(cat),
                Err(error) => {
                    warnings.push(format!(
                        "Column labels cannot read recorded content: {error}"
                    ));
                    None
                }
            }
        });
        if cat.is_none() {
            warnings.push("Game label translations are unavailable without the recorded content stack; plain column names are UI descriptions".into());
        }
        let columns = registry(cat.as_deref());
        let default_columns = vec![
            ScoreboardColumnKey::Factories,
            ScoreboardColumnKey::ArmySize,
            ScoreboardColumnKey::Territory,
            ScoreboardColumnKey::Casualties,
            ScoreboardColumnKey::Planes,
            ScoreboardColumnKey::Ships,
        ];
        if params.tags.is_empty() && params.top.is_none() {
            return checked_reply(ScoreboardReply {
                branch_id: params.branch_id.clone(),
                default_columns,
                default_sort: ScoreboardColumnKey::Factories,
                columns,
                series: Vec::new(),
                warnings,
            });
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Scoreboard cancelled",
            ));
        }
        let measures: Vec<_> = params
            .columns
            .iter()
            .filter_map(|key| match key {
                ScoreboardColumnKey::PoliticalPower => {
                    Some(super::country_measure::Measure::PoliticalPower)
                }
                ScoreboardColumnKey::CommandPower => {
                    Some(super::country_measure::Measure::CommandPower)
                }
                _ => None,
            })
            .collect();
        let history = country_history::branch_histories(
            source,
            branch,
            params.columns.contains(&ScoreboardColumnKey::Territory),
            &measures,
            cancelled,
        )?;
        if history.unreadable_territory > 0 {
            warnings.push(format!(
                "{} owned-state count rows were unresolved or ambiguous; their totals are unknown",
                history.unreadable_territory
            ));
        }
        if history.unnamed > 0 {
            warnings.push(format!(
                "{} country-hours have non-UTF-8 tags and cannot be named",
                history.unnamed
            ));
        }
        if history.unresolved_countries > 0 {
            warnings.push(format!("{} country-hours have unresolved country links; their identity and values are unknown", history.unresolved_countries));
        }
        let mut values_by_key = BTreeMap::new();
        let membership = if let Some(top) = &params.top {
            let tags: Vec<_> = history.countries.keys().cloned().collect();
            let found = values(
                source,
                branch,
                top.sort,
                &tags,
                &history,
                cancelled,
                &mut warnings,
            )?;
            let members = ranking::top_membership_exact(
                &ranking_runs(&history, &found),
                top.count as usize,
                |run| (run.0, run.1),
                |run| run.2,
                top.descending,
                cancelled,
            )?;
            values_by_key.insert(top.sort, found);
            Some(members)
        } else {
            None
        };
        let tags: Vec<_> = match &membership {
            Some(members) => members.keys().cloned().collect(),
            None => country_history::selected_aliases(&history, &params.tags),
        };
        for key in &params.columns {
            if !values_by_key.contains_key(key) {
                values_by_key.insert(
                    *key,
                    values(
                        source,
                        branch,
                        *key,
                        &tags,
                        &history,
                        cancelled,
                        &mut warnings,
                    )?,
                );
            }
        }
        let mut series = Vec::new();
        for key in &params.columns {
            fill_read_runs(
                source,
                branch,
                *key,
                &tags,
                values_by_key.get_mut(key).expect("requested column"),
                cancelled,
            )?;
        }
        for tag in tags {
            let Some(country) = history.countries.get(&tag) else {
                continue;
            };
            let member = membership
                .as_ref()
                .and_then(|members| members.get(&tag))
                .cloned();
            let mut shown = Vec::new();
            for key in &params.columns {
                let column = values_by_key
                    .get_mut(key)
                    .and_then(|values| values.remove(&tag))
                    .unwrap_or(ColumnData {
                        key: *key,
                        runs: Vec::new(),
                        read_runs: Vec::new(),
                    });
                let column = match &member {
                    Some(spans) => clip_column(column, spans, cancelled)?,
                    None => column,
                };
                shown.push(column.reply());
            }
            let entry = ScoreboardSeries {
                tag,
                segments: country.segments.as_ref().clone(),
                identities: country.identities.as_ref().clone(),
                membership: member,
                columns: shown,
            };
            series.push(entry);
        }
        warnings.sort();
        warnings.dedup();
        if let Some(cat) = cat {
            warnings.extend(cat.refusal_lines());
        }
        let reply = ScoreboardReply {
            branch_id: params.branch_id.clone(),
            default_columns,
            default_sort: ScoreboardColumnKey::Factories,
            columns,
            series,
            warnings,
        };
        checked_reply(reply)
    }
}

/// Serialize once against the shared budget; no count or scan caps substitute
/// for the size of the exact reply, including a registry-only response.
fn checked_reply(reply: ScoreboardReply) -> io::Result<ScoreboardReply> {
    if json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(io::Error::other(
            "The scoreboard exceeds MAX_PRODUCT_REPLY_BYTES (24 MiB reply budget); choose fewer columns or countries",
        ));
    }
    Ok(reply)
}

#[cfg(test)]
mod tests;
