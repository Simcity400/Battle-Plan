//! An exact table of shared per-country measures. There is no overall score.
//! Factories reuse the game's whole-count parts and count modes.
//! Political/command balances reuse country_measure's stored fixed5 values;
//! stability, war support and nukes reuse their shared arithmetic.
//! Equipment totals reuse the cached production scan and its exact delivered-unit
//! helper, never another production walk. Owned states use the recorded roster
//! count, excluding zero-province templates.
//! This uses the shared unambiguous country-link join. Identities are independent
//! of value coverage, so unknown amounts rank last, never zero.

use super::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, air_force, army_size,
    country_factories::{self, FactoryCount},
    country_groups::HourSpan,
    country_history::{self, CountryIdentityRun, RecordedHistories},
    country_nukes, equipment_produced, navy, validate_tag, war_casualties,
};
#[cfg(test)]
use crate::products::country_factories::FactoryGroupKey;
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
    sync::atomic::AtomicBool,
};
use ts_rs::TS;
#[cfg(test)]
use {
    super::{country_measure::FixedRun, country_stability, equipment},
    std::sync::atomic::Ordering,
};

pub struct Scoreboard;
pub(crate) mod prepared;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum ScoreboardColumnKey {
    Factories,
    ArmySize,
    Territory,
    Casualties,
    CasualtiesDealt,
    CasualtiesTaken,
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
    pub const ALL: [Self; 16] = [
        Self::Factories,
        Self::ArmySize,
        Self::Territory,
        Self::Casualties,
        Self::CasualtiesDealt,
        Self::CasualtiesTaken,
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

    fn unavailable_reason(self) -> Option<&'static str> {
        (self == Self::Casualties)
            .then_some("Choose Casualties dealt or Casualties taken in settings.")
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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Exact signed integer in units of 10^-decimals. None is unknown.
    pub amount: Option<String>,
    pub starts_segment: bool,
    /// Legacy wire name: true marks a partial recorded subtotal, not a lower bound.
    #[ts(optional, as = "Option<_>")]
    pub at_least: bool,
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
/// The owned-state roster count uses the plain UI description "States owned".
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
            unit: "people",
            decimals: 0,
        },
        Casualties => ColumnDescriptor {
            label: "Casualties",
            unit: "people",
            decimals: 0,
        },
        CasualtiesDealt => ColumnDescriptor {
            label: "Casualties dealt",
            unit: "people",
            decimals: 0,
        },
        CasualtiesTaken => ColumnDescriptor {
            label: "Casualties taken",
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

pub(crate) fn registry(catalogue: Option<&Catalogue>) -> Vec<ScoreboardColumn> {
    ScoreboardColumnKey::ALL
        .into_iter()
        .map(|key| {
            let description = descriptor(key);
            let reason = key.unavailable_reason().map(str::to_owned);
            // Political, stability, war-support and command-power headings use
            // their localisation keys; every translation is recorded content.
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
                note: match key {
                    ScoreboardColumnKey::ArmySize => Some(
                        "Manpower in the field, attributed to each division's owner, as in the Army size widget's default.".into(),
                    ),
                    ScoreboardColumnKey::CasualtiesDealt => Some(
                        "Recorded casualties dealt across land, naval and air. † marks a partial recorded subtotal; missing history may omit positive losses or negative corrections.".into(),
                    ),
                    ScoreboardColumnKey::CasualtiesTaken => Some(
                        "Recorded casualties taken across land, naval and air with a known attacker, as in the Casualties widget's default. Losses without a recorded attacker are excluded; † marks a partial recorded subtotal.".into(),
                    ),
                    _ => None,
                },
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ValueRun {
    first_hour: i32,
    last_hour: i32,
    amount: Option<i128>,
    starts_segment: bool,
    at_least: bool,
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
                    at_least: run.at_least,
                })
                .collect(),
            read_runs: self.read_runs,
        }
    }
}

#[cfg(test)]
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
                at_least: false,
            })
        })
        .collect()
}

/// Merge sorted value coverage and held-read intervals in one forward walk.
/// The read cursor stays on a carried interval spanning several value runs.
#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
type ColumnValues = BTreeMap<String, ColumnData>;

fn factory_warnings(scanned: &country_factories::ScannedFactories, warnings: &mut Vec<String>) {
    if scanned.unreadable > 0 {
        warnings.push(format!("{} factory rows were unresolved, ambiguous or outside the game's displayed whole-count domain", scanned.unreadable));
    }
}
fn nuke_warnings(scanned: &country_nukes::ScannedNukes, warnings: &mut Vec<String>) {
    for (kind, count) in &scanned.refused {
        warnings.push(format!(
            "{count} unsupported nuclear array entries of recorded ordinal {kind}"
        ));
    }
}
fn casualty_warnings(incomplete: bool, warnings: &mut Vec<String>) {
    if incomplete {
        warnings.push("Casualty history has gaps; wars entirely inside a gap may be missing. Partial recorded subtotals are marked †. Missing history may omit positive losses or negative corrections.".into());
    }
}

#[cfg(test)]
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
    if matches!(
        key,
        ScoreboardColumnKey::ArmySize | ScoreboardColumnKey::Planes | ScoreboardColumnKey::Ships
    ) {
        if tags.is_empty() {
            return Ok(out);
        }
        for tag in tags {
            out.insert(
                tag.clone(),
                ColumnData {
                    key,
                    runs: Vec::new(),
                    read_runs: Vec::new(),
                },
            );
        }
        let mut visit = |tag: &str,
                         first_hour: i32,
                         last_hour: i32,
                         starts_segment: bool,
                         amount: Option<i128>| {
            let runs = &mut out.get_mut(tag).expect("requested force tag").runs;
            if let Some(last) = runs.last_mut()
                && !starts_segment
                && last.last_hour.checked_add(1) == Some(first_hour)
                && last.amount == amount
            {
                last.last_hour = last_hour;
            } else {
                runs.push(ValueRun {
                    first_hour,
                    last_hour,
                    amount,
                    starts_segment,
                    at_least: false,
                });
            }
            Ok(())
        };
        match key {
            ScoreboardColumnKey::ArmySize => army_size::visit_army_runs(
                source,
                branch,
                tags,
                army_size::ArmyMeasure::Manpower,
                army_size::ArmyAttribution::Owner,
                cancelled,
                &mut visit,
            )?,
            ScoreboardColumnKey::Planes => air_force::visit_plane_runs(
                source,
                branch,
                tags,
                cancelled,
                |tag, first, last, starts, amount| {
                    visit(tag, first, last, starts, amount.map(i128::from))
                },
            )?,
            ScoreboardColumnKey::Ships => navy::visit_ship_runs(
                source,
                branch,
                tags,
                cancelled,
                |tag, first, last, starts, amount| {
                    visit(tag, first, last, starts, amount.map(i128::from))
                },
            )?,
            _ => unreachable!(),
        }
        return Ok(out);
    }
    if let Some(mode) = key.factory_count() {
        let scanned = country_factories::scanned_branch(source, branch, cancelled)?;
        factory_warnings(&scanned, warnings);
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
                            .map(i128::from)
                            .sum::<i128>(),
                    ),
                    at_least: false,
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
                            amount: Some(i128::from(run.value)),
                            starts_segment: run.starts_segment,
                            at_least: false,
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
                        amount: run.total.map(i128::from),
                        starts_segment: run.starts_segment,
                        at_least: false,
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
            nuke_warnings(&scanned, warnings);
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
                        amount: run.amount.map(i128::from),
                        starts_segment: run.starts_segment,
                        at_least: false,
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
        ScoreboardColumnKey::CasualtiesDealt | ScoreboardColumnKey::CasualtiesTaken => {
            let scanned = war_casualties::scanned_branch(source, branch, cancelled)?;
            casualty_warnings(scanned.has_gap(), warnings);
            for tag in tags {
                let mut runs: Vec<ValueRun> = Vec::new();
                scanned.for_each_country_run(tag, cancelled, |run| {
                    let amount = Some(if key == ScoreboardColumnKey::CasualtiesDealt {
                        run.value.dealt
                    } else {
                        run.value.taken
                    });
                    if let Some(last) = runs.last_mut()
                        && !run.starts_segment
                        && last.last_hour.checked_add(1) == Some(run.first_hour)
                        && last.amount == amount
                        && last.at_least == run.value.at_least
                    {
                        last.last_hour = run.last_hour;
                    } else {
                        runs.push(ValueRun {
                            first_hour: run.first_hour,
                            last_hour: run.last_hour,
                            amount,
                            starts_segment: run.starts_segment,
                            at_least: run.value.at_least,
                        });
                    }
                    Ok(())
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
        _ => {}
    }
    Ok(out)
}

/// Supplying dates are needed only for countries returned to the table, never
/// for the whole ranking universe. Shared scans/cache lookups do not re-read data.
#[cfg(test)]
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
        ScoreboardColumnKey::CasualtiesDealt | ScoreboardColumnKey::CasualtiesTaken => {
            let scanned = war_casualties::scanned_branch(source, branch, cancelled)?;
            for tag in tags {
                if let Some(column) = columns.get_mut(tag) {
                    scanned.for_each_country_run(tag, cancelled, |run| {
                        column.read_runs.push(ScoreboardReadRun {
                            first_hour: run.first_hour,
                            last_hour: run.last_hour,
                            read_hour: run.value.read_hour,
                            hourly: false,
                        });
                        Ok(())
                    })?;
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
#[cfg(test)]
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
                        Some(run) if run.first_hour <= hour => {
                            (run.last_hour.min(segment.last_hour), run.amount)
                        }
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

#[cfg(test)]
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
                at_least: run.at_least,
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

fn history_warnings(history: &RecordedHistories, warnings: &mut Vec<String>) {
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
        warnings.push(format!(
            "{} country-hours have unresolved country links; their identity and values are unknown",
            history.unresolved_countries
        ));
    }
}

fn metadata(
    source: &Source,
    branch_id: &str,
    context: &LoadContext,
) -> (ScoreboardReply, Option<std::sync::Arc<Catalogue>>) {
    let mut warnings = Vec::new();
    let cat =
        context
            .content
            .as_ref()
            .and_then(|options| match catalogue::catalogue(source, options) {
                Ok(cat) => Some(cat),
                Err(error) => {
                    warnings.push(format!(
                        "Column labels cannot read recorded content: {error}"
                    ));
                    None
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
        ScoreboardColumnKey::CasualtiesDealt,
        ScoreboardColumnKey::CasualtiesTaken,
        ScoreboardColumnKey::Planes,
        ScoreboardColumnKey::Ships,
    ];
    (
        ScoreboardReply {
            branch_id: branch_id.into(),
            default_columns,
            default_sort: ScoreboardColumnKey::Factories,
            columns,
            series: Vec::new(),
            warnings,
        },
        cat,
    )
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
            prepared::ScoreboardPreparedParams::decl(),
            prepared::ScoreboardPreparedReply::decl(),
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

    #[cfg(test)]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let (metadata, cat) = metadata(source, &params.branch_id, context);
        if params.tags.is_empty() && params.top.is_none() {
            return checked_reply(metadata);
        }
        let ScoreboardReply {
            columns,
            default_columns,
            mut warnings,
            ..
        } = metadata;
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
        history_warnings(&history, &mut warnings);
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
