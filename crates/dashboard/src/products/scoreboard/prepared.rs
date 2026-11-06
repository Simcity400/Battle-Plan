//! A moment index and visible cells composed from the existing prepared suppliers.
use super::*;
use crate::products::army_size;
use crate::{
    metadata::check_cancel,
    prepared::series::{self, Reader},
};
use dashboard_base::prepared::{engine, published};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::PoisonError,
};

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScoreboardPreparedParams {
    pub selection: ScoreboardParams,
    pub hour: i32,
    pub sort: ScoreboardColumnKey,
    pub descending: bool,
    /// The index contains identities and the sorting column. Other cells are paged by tag.
    pub index: bool,
    pub bars: bool,
    /// The hour of the complete index the asker holds: the reply's rows are then the countries'
    /// that differ from it there.
    #[serde(default)]
    #[ts(optional)]
    pub since: Option<i32>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardPreparedReply {
    pub hour: i32,
    pub data: ScoreboardReply,
    /// Exact positive maxima over the selected table, independent of its mounted rows.
    pub maxima: BTreeMap<ScoreboardColumnKey, String>,
    /// The hour the rows are read against: a country's row there, at `hour`, stands unless a
    /// row of it is sent or it ended. None for every row.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub since: Option<i32>,
    /// The countries with a row at `since` and none at `hour`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub ended: Vec<String>,
}
#[derive(Clone, Default)]
struct Cell {
    amount: Option<i128>,
    read_hour: Option<i32>,
    at_least: bool,
}
impl Cell {
    fn reply(
        self,
        key: ScoreboardColumnKey,
        hour: i32,
        with_reads: bool,
    ) -> ScoreboardColumnSeries {
        let recorded = self.read_hour.is_some() || !with_reads && self.amount.is_some();
        ColumnData {
            key,
            runs: recorded
                .then_some(ValueRun {
                    first_hour: hour,
                    last_hour: hour,
                    amount: self.amount,
                    starts_segment: false,
                    at_least: self.at_least,
                })
                .into_iter()
                .collect(),
            read_runs: self
                .read_hour
                .filter(|_| with_reads)
                .map(|read_hour| ScoreboardReadRun {
                    first_hour: hour,
                    last_hour: hour,
                    read_hour,
                    hourly: read_hour == hour,
                })
                .into_iter()
                .collect(),
        }
        .reply()
    }
}

/// The product whose coverage the pages a column reads wait for
/// (`pipeline::product_coverage`): its supplier's own while that family is
/// prepared on its own (territory's with the identity history's), the
/// branch's otherwise. None for a column that reads no pages.
fn gate(column: ScoreboardColumnKey) -> Option<&'static str> {
    use ScoreboardColumnKey::*;
    if column.factory_count().is_some() {
        return Some("country-factories");
    }
    Some(match column {
        ArmySize => "army-size",
        Planes => "air-force-size",
        Ships => "navy-size",
        PoliticalPower | CommandPower => "country-measure",
        Stability | WarSupport => "country-stability",
        Nukes => "country-nukes",
        Territory => <crate::products::territory::Territory as Product>::KEY,
        CasualtiesDealt | CasualtiesTaken => "war-casualties",
        EquipmentProduced => "production-select",
        Casualties | Factories | FactoriesBuilt | FactoriesTopbar => return None,
    })
}

/// Each requested column follows its supplier's prefix. Exact frame columns
/// can still answer at the moment while the other columns are preparing.
fn prefix(
    source: &Source,
    context: &LoadContext,
    branch: u64,
    columns: &[ScoreboardColumnKey],
    hour: i32,
    accepted: Option<crate::prepared::coverage::Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<(
    crate::prepared::prefix::Prefix,
    BTreeSet<ScoreboardColumnKey>,
)> {
    let mut joint = crate::prepared::prefix::extent(source, branch, accepted)?;
    let mut preparing = BTreeSet::new();
    let mut suppliers = BTreeMap::new();
    for &column in columns {
        let Some(product) = gate(column) else {
            continue;
        };
        if let std::collections::btree_map::Entry::Vacant(entry) = suppliers.entry(product) {
            let accepted = match crate::prepared::pipeline::product_coverage(
                source,
                context.content.as_ref(),
                branch,
                product,
                cancelled,
            ) {
                Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
                other => other?,
            };
            entry.insert(crate::prepared::prefix::extent(source, branch, accepted)?);
        }
        let coverage = &suppliers[product];
        joint.last_hour = joint.last_hour.min(coverage.last_hour);
        joint.complete &= coverage.complete;
        if !coverage.complete && hour > coverage.last_hour {
            preparing.insert(column);
        }
    }
    Ok((
        crate::prepared::prefix::Prefix::new(source, branch, joint, Some(hour)),
        preparing,
    ))
}

fn amounts(
    reader: &mut Reader<'_>,
    keys: &[Option<u64>],
    hour: i32,
) -> io::Result<Vec<Option<i128>>> {
    let values = reader.values_at(
        &keys
            .iter()
            .map(|key| key.unwrap_or(u64::MAX))
            .collect::<Vec<_>>(),
        hour,
    )?;
    Ok(values
        .into_iter()
        .zip(keys)
        .map(|(value, key)| key.and(value))
        .collect())
}

#[allow(clippy::too_many_arguments)]
fn cells(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
    store: &store::Store,
    source_key: &store::SourceKey,
    branch: u64,
    column: ScoreboardColumnKey,
    rows: &[(String, CountryIdentityRun)],
    hour: i32,
    with_reads: bool,
    warnings: &mut Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<Cell>>> {
    use ScoreboardColumnKey::*;
    check_cancel(cancelled)?;
    if rows.is_empty() {
        return Ok(Some(Vec::new()));
    }
    if column == Casualties {
        return Ok(Some(vec![Cell::default(); rows.len()]));
    }
    if column == EquipmentProduced {
        let selection = equipment_produced::ProducedParams {
            branch_id: branch.to_string(),
            tags: rows.iter().map(|row| row.0.clone()).collect(),
            top: None,
            measure: equipment_produced::ProducedMeasure::Total,
            count: equipment_produced::ProducedCount::Units,
            grouping: equipment_produced::ProducedGrouping::Variant,
            included: Vec::new(),
            refits: false,
        };
        let mut page = crate::prepared_production::select_at(
            source,
            content,
            store,
            source_key,
            hour,
            &selection,
            0,
            rows.len(),
            false,
            cancelled,
        )?;
        if page.is_none() {
            // A gap has no endpoint. The prepared catalogue still supplies the
            // native recording-wide diagnostics without scanning or inventing cells.
            let metadata = equipment_produced::ProducedParams {
                tags: Vec::new(),
                ..selection
            };
            page = crate::prepared_production::select_at(
                source, content, store, source_key, hour, &metadata, 0, 0, false, cancelled,
            )?;
        }
        let found: BTreeMap<_, _> = page
            .into_iter()
            .flat_map(|page| {
                warnings.extend(page.warnings);
                page.countries
            })
            .map(|country| {
                (
                    (country.tag.clone(), country.slot, country.country_id),
                    country,
                )
            })
            .collect();
        return rows
            .iter()
            .map(|(tag, identity)| {
                let country = found.get(&(tag.clone(), identity.slot, identity.country_id));
                Ok(Cell {
                    amount: country
                        .and_then(|country| country.total.as_deref())
                        .map(str::parse::<i128>)
                        .transpose()
                        .map_err(|error| crate::invalid(error.to_string()))?,
                    read_hour: country
                        .filter(|_| with_reads)
                        .map(|country| country.read_hour),
                    at_least: false,
                })
            })
            .collect::<io::Result<Vec<_>>>()
            .map(Some);
    }
    if matches!(column, CasualtiesDealt | CasualtiesTaken) {
        let Some((keys, metadata)) =
            series::read_catalogue::<Value>(store, source_key, "casualty", branch)?
        else {
            return Ok(None);
        };
        casualty_warnings(metadata["incomplete"].as_bool().unwrap_or(false), warnings);
        let selected: Vec<_> = rows
            .iter()
            .map(|(tag, identity)| {
                war_casualties::prepared::actor_key(tag, (identity.slot, identity.country_id))
                    .map(|name| keys.get(&name).copied())
            })
            .collect::<io::Result<_>>()?;
        if with_reads {
            let reached = war_casualties::prepared::read_measures(
                store,
                source_key,
                branch,
                &selected
                    .iter()
                    .map(|key| key.unwrap_or(u64::MAX))
                    .collect::<Vec<_>>(),
                hour,
                cancelled,
            )?;
            return Ok(Some(
                reached
                    .into_iter()
                    .zip(selected)
                    .map(|(value, key)| match value.filter(|_| key.is_some()) {
                        Some(value) => Cell {
                            amount: Some(if column == CasualtiesDealt {
                                value.dealt
                            } else {
                                value.taken
                            }),
                            read_hour: Some(value.read_hour),
                            at_least: value.at_least,
                        },
                        None => Cell::default(),
                    })
                    .collect(),
            ));
        }
        let channel = usize::from(column == CasualtiesTaken);
        let mut values = Reader::new(
            store,
            source_key,
            format!("casualty-values-{branch}-{channel}"),
            cancelled,
        )?;
        return Ok(Some(
            amounts(&mut values, &selected, hour)?
                .into_iter()
                .map(|amount| Cell {
                    amount,
                    read_hour: None,
                    at_least: false,
                })
                .collect(),
        ));
    }
    Err(crate::invalid("Missing prepared scoreboard supplier"))
}

/// Each family publication's key reads at the hours the scoreboard asked, with the hours each
/// reads the same ([`engine::held_at`]): an hour among them is answered without reading again.
#[derive(Default)]
struct HeldReads(HashMap<(u64, &'static str), FamilyReads>);

#[derive(Default)]
struct FamilyReads {
    /// The publication and the bound the reads are of.
    stamp: Option<(([u64; 3], u64), Option<i32>)>,
    /// Each key's value, with the first and last hours it reads the same.
    reads: HashMap<u64, (i128, i32, i32)>,
}

/// `cells` of `family`'s publication `opened` at `hour`, each key read through the reads kept
/// of that publication.
#[allow(clippy::too_many_arguments)]
fn held_cells<T>(
    source: &Source,
    branch: u64,
    family: &'static str,
    opened: &published::Opened,
    hour: i32,
    cancelled: &AtomicBool,
    cells: impl FnOnce(
        &dashboard_series::family::Directory,
        &mut dyn FnMut(u64) -> io::Result<Option<i128>>,
    ) -> io::Result<T>,
) -> io::Result<T> {
    let state = source.state::<HeldReads>();
    // Held while it is read, so the requests asking at once read each key once.
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    let held = state.0.entry((branch, family)).or_default();
    let stamp = (opened.stamp(), opened.through());
    if held.stamp != Some(stamp) {
        held.stamp = Some(stamp);
        held.reads.clear();
    }
    let reads = &mut held.reads;
    opened.read(cancelled, |snapshot, directory| {
        cells(directory, &mut |key| {
            if let Some(&(value, first, last)) = reads.get(&key)
                && first <= hour
                && hour <= last
            {
                return Ok(Some(value));
            }
            let (value, (first, last)) = engine::held_at(snapshot, key, hour)?;
            match value {
                Some(value) => {
                    reads.insert(key, (value, first, last));
                }
                None => {
                    reads.remove(&key);
                }
            }
            Ok(value)
        })
    })
}

/// Stability or war support at `hour` from the stability family's publication, dated by
/// each value's supplying read; none before the family publishes the branch.
fn stability_cells(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
    branch: u64,
    column: ScoreboardColumnKey,
    rows: &[(String, CountryIdentityRun)],
    (hour, with_reads): (i32, bool),
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<Cell>>> {
    use crate::products::country_stability;
    let measure = if column == ScoreboardColumnKey::Stability {
        "stability"
    } else {
        "warSupport"
    };
    let Some(opened) = published::open(
        source,
        content,
        "stability",
        country_stability::emit::SCHEMA,
        branch,
    )?
    else {
        return Ok(None);
    };
    let tags: Vec<&str> = rows.iter().map(|(tag, _)| tag.as_str()).collect();
    let found = held_cells(
        source,
        branch,
        "stability",
        &opened,
        hour,
        cancelled,
        |directory, read| country_stability::query::cells_by(directory, &tags, measure, read),
    )?;
    Ok(Some(
        found
            .into_iter()
            .map(|(amount, read)| Cell {
                amount,
                read_hour: read
                    .filter(|_| with_reads && amount.is_some())
                    .map(|read| read as i32),
                at_least: false,
            })
            .collect(),
    ))
}

/// Political or command power at `hour` from the balances' publication, dated `hour` where the
/// country held a value then; none before the family publishes the branch.
fn measure_cells(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
    branch: u64,
    column: ScoreboardColumnKey,
    rows: &[(String, CountryIdentityRun)],
    (hour, with_reads): (i32, bool),
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<Cell>>> {
    use crate::products::country_balances;
    check_cancel(cancelled)?;
    if rows.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let measure = if column == ScoreboardColumnKey::PoliticalPower {
        "politicalPower"
    } else {
        "commandPower"
    };
    let Some(opened) = published::open(
        source,
        content,
        country_balances::NAME,
        country_balances::emit::SCHEMA,
        branch,
    )?
    else {
        return Ok(None);
    };
    let tags: Vec<&str> = rows.iter().map(|(tag, _)| tag.as_str()).collect();
    let found = held_cells(
        source,
        branch,
        "balances",
        &opened,
        hour,
        cancelled,
        |directory, read| country_balances::query::cells_by(directory, &tags, measure, read),
    )?;
    Ok(Some(
        found
            .into_iter()
            .map(|amount| Cell {
                amount,
                read_hour: amount.filter(|_| with_reads).map(|_| hour),
                at_least: false,
            })
            .collect(),
    ))
}

/// Owner manpower at `hour` from the army family's publication, dated `hour` where the
/// country's army was known then; none before the family publishes the branch.
fn army_cells(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
    branch: u64,
    rows: &[(String, CountryIdentityRun)],
    (hour, with_reads): (i32, bool),
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<Cell>>> {
    let Some(opened) = published::open(source, content, "army", army_size::emit::SCHEMA, branch)?
    else {
        return Ok(None);
    };
    let tags: Vec<&str> = rows.iter().map(|(tag, _)| tag.as_str()).collect();
    let found = held_cells(
        source,
        branch,
        "army",
        &opened,
        hour,
        cancelled,
        |directory, read| army_size::query::cells_by(directory, &tags, read),
    )?;
    Ok(Some(
        found
            .into_iter()
            .map(|(amount, known)| Cell {
                amount,
                read_hour: (with_reads && known).then_some(hour),
                at_least: false,
            })
            .collect(),
    ))
}

/// Owned states at `hour` from territory's publication, the native counter its holder read,
/// dated `hour` where it held one; none before the family publishes the branch.
fn territory_cells(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
    branch: u64,
    rows: &[(String, CountryIdentityRun)],
    (hour, with_reads): (i32, bool),
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<Cell>>> {
    use crate::products::territory;
    check_cancel(cancelled)?;
    if rows.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let Some(opened) = published::open(
        source,
        content,
        "territory",
        territory::emit::SCHEMA,
        branch,
    )?
    else {
        return Ok(None);
    };
    let tags: Vec<&str> = rows.iter().map(|(tag, _)| tag.as_str()).collect();
    let found = held_cells(
        source,
        branch,
        "territory",
        &opened,
        hour,
        cancelled,
        |directory, read| territory::query::owned_states_by(directory, &tags, read),
    )?;
    Ok(Some(
        found
            .into_iter()
            .map(|amount| Cell {
                amount,
                read_hour: amount.filter(|_| with_reads).map(|_| hour),
                at_least: false,
            })
            .collect(),
    ))
}

/// Roster planes at `hour` from the air family's publication, dated `hour` where the country's
/// census was read then; none before the family publishes the branch.
fn air_cells(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
    branch: u64,
    rows: &[(String, CountryIdentityRun)],
    (hour, with_reads): (i32, bool),
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<Cell>>> {
    check_cancel(cancelled)?;
    if rows.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let Some(opened) = published::open(source, content, "air", air_force::emit::SCHEMA, branch)?
    else {
        return Ok(None);
    };
    let tags: Vec<&str> = rows.iter().map(|(tag, _)| tag.as_str()).collect();
    let (found, scanned) = held_cells(
        source,
        branch,
        "air",
        &opened,
        hour,
        cancelled,
        |directory, read| {
            Ok((
                air_force::query::cells_by(directory, &tags, read)?,
                air_force::query::scanned(directory),
            ))
        },
    )?;
    crate::products::require_displayable_tags(&scanned.undisplayable_tags)?;
    Ok(Some(
        found
            .into_iter()
            .map(|(amount, read)| Cell {
                amount,
                read_hour: (with_reads && read).then_some(hour),
                at_least: false,
            })
            .collect(),
    ))
}

/// Roster ships at `hour` from the navy family's publication, dated `hour` where the country's
/// census was read then; none before the family publishes the branch.
fn navy_cells(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
    branch: u64,
    rows: &[(String, CountryIdentityRun)],
    (hour, with_reads): (i32, bool),
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<Cell>>> {
    check_cancel(cancelled)?;
    if rows.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let Some(opened) = published::open(source, content, "navy", navy::emit::SCHEMA, branch)?
    else {
        return Ok(None);
    };
    let tags: Vec<&str> = rows.iter().map(|(tag, _)| tag.as_str()).collect();
    let (found, scanned) = held_cells(
        source,
        branch,
        "navy",
        &opened,
        hour,
        cancelled,
        |directory, read| {
            Ok((
                navy::query::cells_by(directory, &tags, read)?,
                navy::query::scanned(directory),
            ))
        },
    )?;
    crate::products::require_displayable_tags(&scanned.undisplayable_tags)?;
    Ok(Some(
        found
            .into_iter()
            .map(|(amount, read)| Cell {
                amount,
                read_hour: (with_reads && read).then_some(hour),
                at_least: false,
            })
            .collect(),
    ))
}

/// Nuclear totals at `hour` from the nukes family's publication, dated `hour` where the country's
/// stockpile was read then; none before the family publishes the branch.
fn nukes_cells(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
    branch: u64,
    rows: &[(String, CountryIdentityRun)],
    (hour, with_reads): (i32, bool),
    warnings: &mut Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<Cell>>> {
    check_cancel(cancelled)?;
    if rows.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let Some(opened) = published::open(
        source,
        content,
        "nukes",
        country_nukes::emit::SCHEMA,
        branch,
    )?
    else {
        return Ok(None);
    };
    let tags: Vec<&str> = rows.iter().map(|(tag, _)| tag.as_str()).collect();
    let (found, scanned) = held_cells(
        source,
        branch,
        "nukes",
        &opened,
        hour,
        cancelled,
        |directory, read| {
            Ok((
                country_nukes::query::cells_by(directory, &tags, read)?,
                country_nukes::query::scanned(directory),
            ))
        },
    )?;
    nuke_warnings(&scanned, warnings);
    Ok(Some(
        found
            .into_iter()
            .map(|(amount, read)| Cell {
                amount,
                read_hour: (with_reads && read).then_some(hour),
                at_least: false,
            })
            .collect(),
    ))
}

/// Factory counts over every group at `hour` from the factories family's publication, dated
/// `hour` where the country's values were known then; none before the family publishes the
/// branch.
#[allow(clippy::too_many_arguments)]
fn factory_cells(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
    branch: u64,
    column: ScoreboardColumnKey,
    rows: &[(String, CountryIdentityRun)],
    (hour, with_reads): (i32, bool),
    warnings: &mut Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<Cell>>> {
    check_cancel(cancelled)?;
    let mode = column
        .factory_count()
        .ok_or_else(|| crate::invalid("Missing factory scoreboard column"))?;
    if rows.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let Some(opened) = published::open(
        source,
        content,
        "factories",
        country_factories::emit::SCHEMA,
        branch,
    )?
    else {
        return Ok(None);
    };
    let tags: Vec<&str> = rows.iter().map(|(tag, _)| tag.as_str()).collect();
    let (found, scanned) = held_cells(
        source,
        branch,
        "factories",
        &opened,
        hour,
        cancelled,
        |directory, read| {
            Ok((
                country_factories::query::cells_by(directory, &tags, mode, read)?,
                country_factories::query::scanned(directory),
            ))
        },
    )?;
    factory_warnings(&scanned, warnings);
    Ok(Some(
        found
            .into_iter()
            .map(|amount| Cell {
                amount,
                read_hour: amount.filter(|_| with_reads).map(|_| hour),
                at_least: false,
            })
            .collect(),
    ))
}

/// A moment beyond accepted preparation, read from its own frame: the identity join
/// preparation runs on that frame. An unrecorded hour reads nothing.
fn sample_moment(
    source: &Source,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<country_history::RecordedHistories> {
    let frame = crate::prepared::query::progressive::frame_at(
        source,
        branch,
        hour,
        &[
            replay_data::rows::TableId::Countries,
            replay_data::rows::TableId::CountryLinks,
        ],
        cancelled,
    )?;
    let mut identities = country_history::HistoryScan::new(false, &[]);
    if let Some(frame) = &frame {
        identities.observe(frame);
    }
    check_cancel(cancelled)?;
    Ok(identities.finish())
}

pub(crate) fn query(
    source: &Source,
    value: Value,
    context: &LoadContext,
    store: &store::Store,
    source_key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let mut params: ScoreboardPreparedParams = serde_json::from_value(value)?;
    let since = params.since.take();
    let mut data = answer(source, &params, context, store, source_key, cancelled)?;
    // Only an index is read against the hour held, and only where that hour's rows were whole.
    let Some(since) = since.filter(|_| params.index && complete(&data)) else {
        return Ok(data);
    };
    let before = answer(
        source,
        &ScoreboardPreparedParams {
            hour: since,
            ..params.clone()
        },
        context,
        store,
        source_key,
        cancelled,
    )?;
    if complete(&before) {
        against(&mut data, &before, since);
    }
    Ok(data)
}

/// Whether `data` answers every row and cell of its hour, as published for good.
fn complete(data: &Value) -> bool {
    data.as_object().is_some_and(|reply| {
        ["pending", "coverage", "approximate", "preparing"]
            .iter()
            .all(|field| !reply.contains_key(*field))
    })
}

/// A row read at the moment `from` as the same row reads at `to`: its spans that hour, and a
/// read supplied every hour supplied then.
fn restamped(row: &Value, from: i32, to: i32) -> Value {
    let mut row = row.clone();
    let at = |span: &mut Value| {
        if let Some(span) = span.as_object_mut() {
            for field in ["firstHour", "lastHour"] {
                if span.get(field).and_then(Value::as_i64) == Some(i64::from(from)) {
                    span.insert(field.into(), to.into());
                }
            }
            if span.get("hourly") == Some(&Value::Bool(true)) {
                span.insert("readHour".into(), to.into());
            }
        }
    };
    for field in ["segments", "identities", "membership"] {
        if let Some(spans) = row.get_mut(field).and_then(Value::as_array_mut) {
            spans.iter_mut().for_each(at);
        }
    }
    if let Some(columns) = row.get_mut("columns").and_then(Value::as_array_mut) {
        for column in columns {
            for field in ["runs", "readRuns"] {
                if let Some(spans) = column.get_mut(field).and_then(Value::as_array_mut) {
                    spans.iter_mut().for_each(at);
                }
            }
        }
    }
    row
}

/// `data` read against `before`, the whole reply at `since`: only the rows that differ from
/// theirs there, and the countries whose rows ended.
fn against(data: &mut Value, before: &Value, since: i32) {
    let Some(hour) = data["hour"].as_i64().and_then(|hour| i32::try_from(hour).ok()) else {
        return;
    };
    let held: HashMap<&str, &Value> = before["data"]["series"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| Some((row["tag"].as_str()?, row)))
        .collect();
    let Some(rows) = data["data"]["series"].as_array_mut() else {
        return;
    };
    let present: HashSet<String> = rows
        .iter()
        .filter_map(|row| row["tag"].as_str().map(str::to_owned))
        .collect();
    rows.retain(|row| {
        row["tag"]
            .as_str()
            .and_then(|tag| held.get(tag))
            .is_none_or(|then| restamped(then, since, hour) != *row)
    });
    let mut ended: Vec<&str> = held
        .keys()
        .copied()
        .filter(|tag| !present.contains(*tag))
        .collect();
    ended.sort_unstable();
    data["since"] = since.into();
    data["ended"] = ended.into();
}

/// [`query`]'s whole reply to `params` at its hour.
fn answer(
    source: &Source,
    params: &ScoreboardPreparedParams,
    context: &LoadContext,
    store: &store::Store,
    source_key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    Scoreboard::validate(&params.selection).map_err(|error| crate::invalid(error.to_string()))?;
    let branch: u64 = params
        .selection
        .branch_id
        .parse::<u64>()
        .map_err(|error| crate::invalid(error.to_string()))?;
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|item| item.id == branch)
    {
        return Err(crate::invalid(
            "Prepared scoreboard branch is not in this recording",
        ));
    }
    let (mut reply, catalogue) = metadata(source, &params.selection.branch_id, context);
    let mut maxima = BTreeMap::new();
    let mut marking = None;
    if !params.selection.tags.is_empty() || params.selection.top.is_some() {
        check_cancel(cancelled)?;
        if params.sort.unavailable_reason().is_some() {
            return Err(crate::invalid("Invalid scoreboard ranking column"));
        }
        if let Some(top) = &params.selection.top
            && (top.sort != params.sort || top.descending != params.descending || !params.index)
        {
            return Err(crate::invalid("Invalid scoreboard index selection"));
        }
        // The coverage every column's family and the identities share, which a native store
        // publishes where the walk's own never is.
        let accepted = match crate::prepared::pipeline::product_coverage(
            source,
            context.content.as_ref(),
            branch,
            <Scoreboard as Product>::KEY,
            cancelled,
        ) {
            Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
            other => other?,
        };
        let complete = accepted.as_ref().is_some_and(|coverage| coverage.complete);
        // Beyond accepted preparation, or beyond the coverage of a supplier
        // whose pages a needed column reads (one prepared on its own may be
        // behind the branch), a column stays pending.
        let needed: Vec<ScoreboardColumnKey> = if params.index {
            std::iter::once(params.sort)
                .chain(
                    params
                        .selection
                        .columns
                        .iter()
                        .copied()
                        .filter(|_| params.bars),
                )
                .collect()
        } else {
            params.selection.columns.clone()
        };
        let (mut prefix, preparing) = prefix(
            source,
            context,
            branch,
            &needed,
            params.hour,
            accepted.clone(),
            cancelled,
        )?;
        if params.index && preparing.contains(&params.sort) {
            let mut data = serde_json::to_value(ScoreboardPreparedReply {
                hour: params.hour,
                data: checked_reply(reply)?,
                maxima,
                since: None,
                ended: Vec::new(),
            })?;
            prefix.mark(&mut data)?;
            data["preparing"] = serde_json::to_value(preparing)?;
            return Ok(data);
        }
        let sampled = if prefix.coverage.complete || params.hour <= prefix.coverage.last_hour {
            None
        } else {
            Some(sample_moment(source, branch, params.hour, cancelled)?)
        };
        if preparing.is_empty() {
            prefix.answered(&[HourSpan {
                first_hour: params.hour,
                last_hour: params.hour,
            }]);
        }
        if !prefix.coverage.complete {
            marking = Some((prefix, preparing.clone()));
        }
        let history = if complete {
            let Some(history) = country_history::prepared_branch_histories(
                source,
                branch,
                context.content.as_ref(),
                cancelled,
            )?
            else {
                return Ok(Value::Null);
            };
            history
        } else if let Some(history) = country_history::prepared_branch_histories(
            source,
            branch,
            context.content.as_ref(),
            cancelled,
        )? {
            // The whole branch's identities, prepared before the other columns.
            history
        } else if params.selection.top.is_none()
            && params.index
            && !params.selection.tags.is_empty()
        {
            // A tag selection of the index names every holder the tags ever
            // had, which only the whole branch's identities know.
            return Ok(Value::Null);
        } else {
            // The accepted prefix's identities, joined with the sampled frame's
            // own identity join at the moment.
            type Identities = BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>;
            let exact: Identities = sampled
                .as_ref()
                .map(|moment| {
                    moment
                        .countries
                        .iter()
                        .map(|(tag, country)| {
                            (
                                (tag.clone(), String::new()),
                                country
                                    .identities
                                    .iter()
                                    .map(|run| {
                                        (run.first_hour, run.last_hour, run.slot, run.country_id)
                                    })
                                    .collect(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut joined = crate::prepared::query::progressive::histories(
                store,
                source_key,
                branch,
                accepted.as_ref(),
                &BTreeMap::new(),
                &exact,
                source.replay_ref(),
                &params.selection.tags,
                cancelled,
            )?;
            if let Some(moment) = &sampled {
                joined.unreadable_territory += moment.unreadable_territory;
                joined.unresolved_countries += moment.unresolved_countries;
                joined.unnamed += moment.unnamed;
            }
            std::sync::Arc::new(joined)
        };
        let column_cells = |column: ScoreboardColumnKey,
                            rows: &[(String, CountryIdentityRun)],
                            with_reads: bool,
                            warnings: &mut Vec<String>|
         -> io::Result<Option<Vec<Cell>>> {
            if preparing.contains(&column) {
                return Ok(Some(vec![Cell::default(); rows.len()]));
            }
            match column {
                _ if column.factory_count().is_some() => factory_cells(
                    source,
                    context.content.as_ref(),
                    branch,
                    column,
                    rows,
                    (params.hour, with_reads),
                    warnings,
                    cancelled,
                ),
                _ if matches!(
                    column,
                    ScoreboardColumnKey::Stability | ScoreboardColumnKey::WarSupport
                ) =>
                {
                    stability_cells(
                        source,
                        context.content.as_ref(),
                        branch,
                        column,
                        rows,
                        (params.hour, with_reads),
                        cancelled,
                    )
                }
                _ if matches!(
                    column,
                    ScoreboardColumnKey::PoliticalPower | ScoreboardColumnKey::CommandPower
                ) =>
                {
                    measure_cells(
                        source,
                        context.content.as_ref(),
                        branch,
                        column,
                        rows,
                        (params.hour, with_reads),
                        cancelled,
                    )
                }
                _ if column == ScoreboardColumnKey::ArmySize => army_cells(
                    source,
                    context.content.as_ref(),
                    branch,
                    rows,
                    (params.hour, with_reads),
                    cancelled,
                ),
                _ if column == ScoreboardColumnKey::Territory => territory_cells(
                    source,
                    context.content.as_ref(),
                    branch,
                    rows,
                    (params.hour, with_reads),
                    cancelled,
                ),
                _ if column == ScoreboardColumnKey::Planes => air_cells(
                    source,
                    context.content.as_ref(),
                    branch,
                    rows,
                    (params.hour, with_reads),
                    cancelled,
                ),
                _ if column == ScoreboardColumnKey::Ships => navy_cells(
                    source,
                    context.content.as_ref(),
                    branch,
                    rows,
                    (params.hour, with_reads),
                    cancelled,
                ),
                _ if column == ScoreboardColumnKey::Nukes => nukes_cells(
                    source,
                    context.content.as_ref(),
                    branch,
                    rows,
                    (params.hour, with_reads),
                    warnings,
                    cancelled,
                ),
                _ => cells(
                    source,
                    context.content.as_ref(),
                    store,
                    source_key,
                    branch,
                    column,
                    rows,
                    params.hour,
                    with_reads,
                    warnings,
                    cancelled,
                ),
            }
        };
        let selected = (params.selection.top.is_none() && params.index)
            .then(|| country_history::selected_identities(&history, &params.selection.tags));
        let current = |(tag, country): (&String, &country_history::RecordedCountry)| {
            let index = country
                .identities
                .partition_point(|identity| identity.first_hour <= params.hour)
                .checked_sub(1)?;
            let identity = &country.identities[index];
            (identity.last_hour >= params.hour
                && selected
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&(identity.slot, identity.country_id))))
            .then(|| (tag.clone(), identity.clone()))
        };
        let mut rows: Vec<_> = if params.index {
            history.countries.iter().filter_map(current).collect()
        } else {
            params
                .selection
                .tags
                .iter()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter_map(|tag| {
                    history
                        .countries
                        .get(tag)
                        .and_then(|country| current((tag, country)))
                })
                .collect()
        };
        let mut reached = BTreeMap::new();
        if params.index {
            let Some(sort) = column_cells(params.sort, &rows, false, &mut reply.warnings)? else {
                return Ok(Value::Null);
            };
            let mut ranked: Vec<_> = rows.into_iter().zip(sort).collect();
            ranked.sort_by(|left, right| {
                ranking::compare(left.1.amount, right.1.amount, params.descending)
                    .then_with(|| left.0.0.cmp(&right.0.0))
            });
            if let Some(top) = &params.selection.top {
                ranked.truncate(top.count as usize);
            }
            let (selected_rows, sort): (Vec<_>, Vec<_>) = ranked.into_iter().unzip();
            rows = selected_rows;
            // Sort values need only amounts; supplying metadata is fetched with visible cells.
            reached.insert(params.sort, sort);
            if params.bars {
                for column in &params.selection.columns {
                    if preparing.contains(column) {
                        continue;
                    }
                    let values = if *column == params.sort {
                        reached.get(column).cloned()
                    } else {
                        column_cells(*column, &rows, false, &mut reply.warnings)?
                    };
                    let Some(values) = values else {
                        return Ok(Value::Null);
                    };
                    maxima.insert(
                        *column,
                        values
                            .iter()
                            .filter_map(|cell| cell.amount)
                            .max()
                            .unwrap_or(0)
                            .max(0)
                            .to_string(),
                    );
                }
            }
        } else {
            for column in &params.selection.columns {
                let Some(values) = column_cells(*column, &rows, true, &mut reply.warnings)? else {
                    return Ok(Value::Null);
                };
                reached.insert(*column, values);
            }
        }
        let span = HourSpan {
            first_hour: params.hour,
            last_hour: params.hour,
        };
        reply.series = rows
            .into_iter()
            .enumerate()
            .map(|(index, (tag, identity))| ScoreboardSeries {
                tag,
                segments: vec![span],
                identities: vec![CountryIdentityRun {
                    first_hour: params.hour,
                    last_hour: params.hour,
                    ..identity
                }],
                membership: params.selection.top.as_ref().map(|_| vec![span]),
                columns: reached
                    .iter()
                    .map(|(&key, cells)| {
                        cells[index].clone().reply(key, params.hour, !params.index)
                    })
                    .collect(),
            })
            .collect();
        history_warnings(&history, &mut reply.warnings);
    }
    reply.warnings.sort();
    reply.warnings.dedup();
    if let Some(catalogue) = catalogue {
        reply.warnings.extend(catalogue.refusal_lines());
    }
    let mut data = serde_json::to_value(ScoreboardPreparedReply {
        hour: params.hour,
        data: checked_reply(reply)?,
        maxima,
        since: None,
        ended: Vec::new(),
    })?;
    if let Some((prefix, preparing)) = marking {
        prefix.mark(&mut data)?;
        if !preparing.is_empty() {
            data["preparing"] = serde_json::to_value(preparing)?;
        }
    }
    Ok(data)
}
