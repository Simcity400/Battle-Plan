//! A moment index and visible cells composed from the existing prepared suppliers.
use super::*;
use crate::products::parts_race_prepared::Family;
use crate::{
    metadata::check_cancel,
    prepared::series::{self, Reader},
};
use serde_json::{Value, json};

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
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardPreparedReply {
    pub hour: i32,
    pub data: ScoreboardReply,
    /// Exact positive maxima over the selected table, independent of its mounted rows.
    pub maxima: BTreeMap<ScoreboardColumnKey, String>,
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
        if !coverage.complete && hour > coverage.last_hour && column.factory_count().is_none() {
            preparing.insert(column);
        }
    }
    Ok((
        crate::prepared::prefix::Prefix::new(source, branch, joint, Some(hour)),
        preparing,
    ))
}

struct Binding {
    family: &'static str,
    section: String,
    field: Option<&'static str>,
    coverage: Option<String>,
}
fn binding(column: ScoreboardColumnKey, branch: u64) -> io::Result<Binding> {
    use ScoreboardColumnKey::*;
    let (family, section, field, coverage) = if let Some(mode) = column.factory_count() {
        let index = country_factories::prepared::channel(&country_factories::FactoryTop {
            count: 1,
            mode,
            groups: Vec::new(),
        });
        (
            "factories",
            format!("factories-values-{branch}-{index}"),
            Some(""),
            None,
        )
    } else {
        match column {
            ArmySize => {
                let index = army_size::channel(
                    army_size::ArmyMeasure::Manpower,
                    army_size::ArmyAttribution::Owner,
                );
                (
                    "army",
                    format!("army-values-{branch}-{index}"),
                    None,
                    Some(format!("army-coverage-{branch}")),
                )
            }
            Planes => {
                let selection = air_force::prepared::Air::selection(
                    json!({"branchId":branch.to_string(),"tags":[],"top":null,"measure":"planes","ownership":"roster"}),
                )?;
                (
                    "air",
                    format!("air-values-{branch}-{}", selection.channel),
                    Some(""),
                    Some(format!("air-mixes-{branch}")),
                )
            }
            Ships => {
                let selection = navy::prepared::Navy::selection(
                    json!({"branchId":branch.to_string(),"tags":[],"top":null,"measure":"ships","ownership":"roster","grouping":"class"}),
                )?;
                (
                    "navy",
                    format!("navy-values-{branch}-{}", selection.channel),
                    Some(""),
                    Some(format!("navy-mixes-{branch}-{}", selection.channel)),
                )
            }
            PoliticalPower => (
                "measure",
                format!("measure-pp-{branch}"),
                Some("politicalPower"),
                None,
            ),
            CommandPower => (
                "measure",
                format!("measure-cp-{branch}"),
                Some("commandPower"),
                None,
            ),
            Stability => (
                "stability",
                format!("stability-{branch}"),
                Some("stability"),
                None,
            ),
            WarSupport => (
                "stability",
                format!("stability-{branch}"),
                Some("warSupport"),
                None,
            ),
            Nukes => (
                "nukes",
                format!("nukes-values-{branch}"),
                Some(""),
                Some(format!("nukes-mixes-{branch}")),
            ),
            Territory => (
                "territory",
                format!(
                    "territory-values-{branch}-{}",
                    crate::products::territory::prepared::NATIVE_OWNED_STATES
                ),
                None,
                None,
            ),
            _ => return Err(crate::invalid("Missing prepared scoreboard supplier")),
        }
    };
    Ok(Binding {
        family,
        section,
        field,
        coverage,
    })
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
            store,
            source_key,
            hour,
            &selection,
            0,
            rows.len(),
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
                store, source_key, hour, &metadata, 0, 0, cancelled,
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
    let binding = binding(column, branch)?;
    let Some((keys, metadata)) =
        series::read_catalogue::<Value>(store, source_key, binding.family, branch)?
    else {
        return Ok(None);
    };
    match column {
        Planes => {
            let scanned: air_force::ScannedAir = serde_json::from_value(metadata[0].clone())?;
            crate::products::require_displayable_tags(&scanned.undisplayable_tags)?;
        }
        Ships => {
            let scanned: navy::ScannedNavy = serde_json::from_value(metadata[0].clone())?;
            crate::products::require_displayable_tags(&scanned.undisplayable_tags)?;
        }
        Factories | FactoriesBuilt | FactoriesTopbar => {
            factory_warnings(&serde_json::from_value(metadata)?, warnings)
        }
        Nukes => nuke_warnings(&serde_json::from_value(metadata)?, warnings),
        _ => {}
    }
    let selected: Vec<_> = rows
        .iter()
        .map(|(tag, _)| {
            keys.get(
                &binding
                    .field
                    .map_or_else(|| tag.clone(), |field| series::field_key(tag, field)),
            )
            .copied()
        })
        .collect();
    let mut reader = Reader::new(store, source_key, binding.section, cancelled)?;
    let values = amounts(&mut reader, &selected, hour)?;
    let reads = if !with_reads {
        vec![None; values.len()]
    } else if matches!(column, Stability | WarSupport) {
        amounts(
            &mut reader,
            &rows
                .iter()
                .map(|row| keys.get(&series::field_key(&row.0, "readHour")).copied())
                .collect::<Vec<_>>(),
            hour,
        )?
    } else if let Some(section) = binding.coverage {
        amounts(
            &mut Reader::new(store, source_key, section, cancelled)?,
            &selected,
            hour,
        )?
        .into_iter()
        .map(|covered| covered.map(|_| i128::from(hour)))
        .collect()
    } else {
        values
            .iter()
            .map(|value| value.map(|_| i128::from(hour)))
            .collect()
    };
    Ok(Some(
        values
            .into_iter()
            .zip(reads)
            .map(|(amount, read)| Cell {
                amount,
                read_hour: read
                    .filter(|_| !matches!(column, Stability | WarSupport) || amount.is_some())
                    .map(|hour| hour as i32),
                at_least: false,
            })
            .collect(),
    ))
}

/// A moment beyond accepted preparation, read from its own frame: the
/// identity join preparation runs on that frame, and the stateless factory
/// sample. An unrecorded hour reads nothing, so every cell stays unknown.
fn sample_moment(
    source: &Source,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<(
    country_history::RecordedHistories,
    crate::prepared::query::progressive::Samples,
)> {
    let frame = crate::prepared::query::progressive::frame_at(
        source,
        branch,
        hour,
        country_factories::prepared::TABLES,
        cancelled,
    )?;
    let mut identities = country_history::HistoryScan::new(false, &[]);
    let mut samples = crate::prepared::query::progressive::Samples::new();
    if let Some(frame) = &frame {
        identities.observe(frame);
        samples = country_factories::prepared::sample(frame, None)?;
    }
    check_cancel(cancelled)?;
    Ok((identities.finish(), samples))
}

/// Factory cells from one sampled frame, dated as the prepared channel dates them.
fn sampled_cells(
    samples: &crate::prepared::query::progressive::Samples,
    column: ScoreboardColumnKey,
    rows: &[(String, CountryIdentityRun)],
    hour: i32,
    with_reads: bool,
) -> io::Result<Vec<Cell>> {
    let mode = column
        .factory_count()
        .ok_or_else(|| crate::invalid("Missing sampled scoreboard supplier"))?;
    rows.iter()
        .map(|(tag, identity)| {
            let amount = country_factories::prepared::sampled_count(
                samples,
                tag,
                (identity.slot, identity.country_id),
                mode,
            )?;
            Ok(Cell {
                amount,
                read_hour: amount.filter(|_| with_reads).map(|_| hour),
                at_least: false,
            })
        })
        .collect()
}

pub(crate) fn query(
    source: &Source,
    value: Value,
    context: &LoadContext,
    store: &store::Store,
    source_key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: ScoreboardPreparedParams = serde_json::from_value(value)?;
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
        let accepted = match crate::prepared::pipeline::coverage(
            source,
            context.content.as_ref(),
            branch,
            cancelled,
        ) {
            Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
            other => other?,
        };
        let complete = accepted.as_ref().is_some_and(|coverage| coverage.complete);
        // Beyond accepted preparation, or beyond the coverage of a supplier
        // whose pages a needed column reads (one prepared on its own may be
        // behind the branch), a moment answers only when one frame proves
        // every cell it needs: the native factory counts. Other columns
        // carry cumulative or carried facts and stay pending.
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
                .map(|(moment, _)| {
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
            if let Some((moment, _)) = &sampled {
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
            match &sampled {
                Some((_, samples)) if column.factory_count().is_some() => {
                    sampled_cells(samples, column, rows, params.hour, with_reads).map(Some)
                }
                _ => cells(
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
    })?;
    if let Some((prefix, preparing)) = marking {
        prefix.mark(&mut data)?;
        if !preparing.is_empty() {
            data["preparing"] = serde_json::to_value(preparing)?;
        }
    }
    Ok(data)
}
