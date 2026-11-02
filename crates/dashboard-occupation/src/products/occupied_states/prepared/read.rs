//! What occupation's replies read of the family's series publication ([`columnar::emit`]): a
//! window's states and reads, the manifest's laws and warnings, and the occupiers, their keys,
//! warnings and runs' facts.

use super::*;
use crate::products::occupied_states::columnar::emit::{
    FACTS, FAMILY, Listing, MANIFEST, OBSERVED, STATES, TEXTS, TREND, VALUE_CELLS, kind,
};
use dashboard_base::prepared::engine;
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot, Span,
    codec::Binary,
    family::{Directory, Scalar},
    query::EventView,
};

fn malformed() -> io::Error {
    dashboard_core::invalid("Prepared occupation values are not valid")
}

fn selection(key: u64) -> Selection<'static> {
    Selection {
        key,
        membership: None,
    }
}

/// Published text `id`.
fn text(directory: &Directory, id: u64) -> io::Result<String> {
    let bytes = directory
        .scalar(TEXTS.checked_add(id).ok_or_else(malformed)?)
        .and_then(Scalar::text)
        .ok_or_else(malformed)?;
    String::from_utf8(bytes.to_vec()).map_err(|_| malformed())
}

/// A state's values as a [`kind::VALUE`] event holds them.
fn values<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    event: EventView,
) -> io::Result<OccupiedValues> {
    let EventView::Family(event) = event else {
        return Err(malformed());
    };
    if event.kind != kind::VALUE || event.fields.len != VALUE_CELLS * Cell::SIZE as u64 {
        return Err(malformed());
    }
    let mut cells = Vec::with_capacity(VALUE_CELLS as usize);
    for at in 0..VALUE_CELLS {
        cells.push(snapshot.field(event.fields, at)?);
    }
    let text = |cell: Cell| match cell {
        Cell::Null => Ok(None),
        Cell::Unsigned(id) => text(directory, id).map(Some),
        _ => Err(malformed()),
    };
    let integer = |cell: Cell| match cell {
        Cell::Integer(value) => i64::try_from(value).map_err(|_| malformed()),
        _ => Err(malformed()),
    };
    let optional = |cell: Cell| match cell {
        Cell::Null => Ok(None),
        cell => integer(cell).map(Some),
    };
    let count = |cell: Cell| {
        optional(cell)?
            .map(|value| i32::try_from(value).map_err(|_| malformed()))
            .transpose()
    };
    Ok(OccupiedValues {
        name: text(cells[0])?,
        name_missing: text(cells[1])?,
        owner: text(cells[2])?,
        occupier: text(cells[3])?,
        occupied: text(cells[4])?,
        resistance_raw: integer(cells[5])?,
        compliance_raw: integer(cells[6])?,
        resistance_speed_raw: integer(cells[7])?,
        compliance_speed_raw: integer(cells[8])?,
        garrison_strength_raw: optional(cells[9])?,
        required_manpower: count(cells[10])?,
        garrison_required_raw: optional(cells[11])?,
        garrison_alert: match cells[12] {
            Cell::Boolean(value) => value,
            _ => return Err(malformed()),
        },
        garrison_red: match cells[13] {
            Cell::Null => None,
            Cell::Boolean(value) => Some(value),
            _ => return Err(malformed()),
        },
        assigned_manpower: count(cells[14])?,
        garrison_missing: text(cells[15])?,
        law: text(cells[16])?,
        law_missing: text(cells[17])?,
    })
}

/// Window `window` of the branch, its hours `interval`, as preparation published them: each
/// state occupied at an hour of it with its values' runs, and the reads supplying them.
pub(super) fn window<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    branch_id: &str,
    window: usize,
    interval: HourSpan,
) -> io::Result<OccupiedStatesReply> {
    let accepted = snapshot.publication().accepted;
    let span = Span {
        first: interval.first_hour,
        last: interval.last_hour,
    }
    .intersect(accepted);
    let mut states = Vec::new();
    let mut observed = Vec::new();
    if let Some(span) = span {
        for &(state, key) in directory.dimensions(FAMILY, STATES) {
            let runs = engine::runs(snapshot, key, span.first, span.last)?;
            let Some(head) = runs.first() else {
                continue;
            };
            // A version's values are an event at its first hour; the first run may start
            // before the window.
            let mut held = BTreeMap::new();
            let event = snapshot
                .latest_event(selection(key), head.span.first, None)?
                .ok_or_else(malformed)?;
            held.insert(head.span.first, values(snapshot, directory, event)?);
            if head.span.first < span.last {
                let keys = [selection(key)];
                let mut events = Vec::new();
                let mut cursor = None;
                loop {
                    let page = snapshot.event_page(
                        EventQuery {
                            selection: &keys,
                            native: Span {
                                first: head.span.first + 1,
                                last: span.last,
                            },
                            as_of: None,
                            order: EventOrder::Native,
                        },
                        cursor,
                        1 << 20,
                        |event| {
                            events.push(event);
                            Ok(())
                        },
                    )?;
                    cursor = page.next;
                    if cursor.is_none() {
                        break;
                    }
                }
                for event in events {
                    let hour = match event {
                        EventView::Family(event) => event.native_hour,
                        EventView::Evidence { .. } => continue,
                    };
                    held.insert(hour, values(snapshot, directory, event)?);
                }
            }
            let runs = runs
                .iter()
                .map(|run| {
                    let (_, value) = held
                        .range(..=run.span.first)
                        .next_back()
                        .ok_or_else(malformed)?;
                    Ok(OccupiedRun {
                        first_hour: run.span.first,
                        last_hour: run.span.last,
                        value: value.clone(),
                    })
                })
                .collect::<io::Result<_>>()?;
            states.push(StateRuns {
                state: u32::try_from(state).map_err(|_| malformed())?,
                runs,
            });
        }
        if let Some(key) = directory.key(FAMILY, OBSERVED, None) {
            for run in engine::runs(snapshot, key, span.first, span.last)? {
                observed.push(OccupiedObservation {
                    first_hour: run.span.first,
                    last_hour: run.span.last,
                    read_hour: i32::try_from(run.value.div_euclid(2)).map_err(|_| malformed())?,
                    complete: run.value.rem_euclid(2) == 1,
                });
            }
        }
    }
    Ok(window_reply_of(
        &states, &observed, branch_id, window, interval,
    ))
}

/// The laws every state of the branch held and the manifest's warnings, as last published.
pub(super) fn listing(directory: &Directory) -> io::Result<Option<Listing>> {
    directory
        .scalar(MANIFEST)
        .and_then(Scalar::text)
        .map(|text| serde_json::from_slice(text).map_err(io::Error::from))
        .transpose()
}

/// The occupier tags by actor id, as the publication names them.
pub(super) fn occupiers(directory: &Directory) -> BTreeMap<String, u64> {
    directory
        .actors
        .iter()
        .enumerate()
        .map(|(actor, named)| (String::from_utf8_lossy(&named.name).into_owned(), actor as u64))
        .collect()
}

/// The trend's warnings, and those of collaboration context.
pub(super) fn trend_warnings(directory: &Directory) -> io::Result<(Vec<String>, Vec<String>)> {
    Ok(directory
        .scalar(TREND)
        .and_then(Scalar::text)
        .map(serde_json::from_slice::<(Vec<String>, Vec<String>)>)
        .transpose()?
        .unwrap_or_default())
}

/// `actor`'s key of `measure`.
pub(super) fn key(directory: &Directory, actor: u64, measure: u64) -> io::Result<u64> {
    directory
        .key(actor, measure, None)
        .ok_or_else(|| dashboard_core::invalid("A prepared occupier's series is missing"))
}

/// The run of `key` holding `hour`, whole.
pub(super) fn run_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Option<dashboard_series::Run>> {
    Ok(engine::run_at(snapshot, key, hour)?.map(|(run, _)| run))
}

/// The fact of `actor`'s run starting by `hour`: its latest [`kind::FACT`] event by then.
pub(super) fn fact_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: u64,
    hour: i32,
) -> io::Result<OccupationTrendRun> {
    let missing = || dashboard_core::invalid("Occupation facts are missing");
    let key = key(directory, actor, FACTS)?;
    let EventView::Family(event) = snapshot
        .latest_event(selection(key), hour, None)?
        .ok_or_else(missing)?
    else {
        return Err(missing());
    };
    if event.kind != kind::FACT || event.fields.len != Cell::SIZE as u64 {
        return Err(missing());
    }
    let Cell::Text(blob) = snapshot.field(event.fields, 0)? else {
        return Err(missing());
    };
    let mut bytes = vec![0; usize::try_from(blob.len).map_err(|_| missing())?];
    snapshot.cell_bytes(blob, 0, &mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Composite facts retain supplying and collaboration boundaries inside an exact window.
pub(super) fn fact_runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    actor: u64,
    from: i32,
    to: i32,
) -> io::Result<Vec<OccupationTrendRun>> {
    let markers = engine::runs(snapshot, key(directory, actor, FACTS)?, from, to)?;
    markers
        .into_iter()
        .map(|marker| {
            let mut run = fact_at(snapshot, directory, actor, marker.span.first)?;
            run.first_hour = marker.span.first;
            run.last_hour = marker.span.last;
            run.starts_segment &= marker.value == i128::from(marker.span.first);
            Ok(run)
        })
        .collect()
}
