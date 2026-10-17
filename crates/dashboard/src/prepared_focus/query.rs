//! Focus queries: the order from the family's pages, progress at the hour
//! from that hour's block read on the progress its first hour starts from.
use super::*;
use crate::prepared::series::Reader;
use replay::container::{BlockChange, BlockIndex};

fn country_reader<'a>(
    store: &'a store::Store,
    source: &'a store::SourceKey,
    branch: u64,
    tag: &str,
    cancelled: &'a AtomicBool,
) -> io::Result<Option<(u64, Reader<'a>)>> {
    check_cancel(cancelled)?;
    let section = format!("focus-{branch}");
    let catalogue = format!("{section}-catalogue");
    let mut files = store.read_many_counted(source, &[(&catalogue, 0), (&section, 0)])?;
    let (count, first) = files.pop().unwrap();
    let Some(file) = files.pop().unwrap().1 else {
        return Ok(None);
    };
    let catalogue: BTreeMap<String, u64> = serde_json::from_reader(io::BufReader::new(file))?;
    Ok(catalogue.get(tag).copied().map(|actor| {
        (
            actor,
            Reader::from_first_file(store, source, section, count, first, cancelled),
        )
    }))
}

fn pages(first: u64, rows: u64) -> Pages {
    Pages {
        sampled: false,
        first,
        count: rows.div_ceil(PAGE_ROWS as u64),
        rows,
    }
}

/// The order event holding at `hour` while `tag` is observed there, with its
/// ordinal and presence run.
fn order_at(
    reader: &mut Reader<'_>,
    actor: u64,
    hour: i32,
) -> io::Result<Option<(i32, u64, OrderEvent, series::Run)>> {
    let Some(presence) = reader.value_at(actor, hour)? else {
        return Ok(None);
    };
    let Some((first, row)) = reader.latest_event(actor, hour)? else {
        return Ok(None);
    };
    let ordinal = reader
        .event_count_through(actor, hour)?
        .checked_sub(1)
        .ok_or_else(|| crate::invalid("Prepared focus order is missing"))?;
    Ok(Some((
        first,
        ordinal,
        serde_json::from_value(row)?,
        presence,
    )))
}

/// A country's focus at `hour`, None where it was not observed (or not yet
/// prepared there).
pub fn query_hour(
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    tag: &str,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<Option<Hour>> {
    check_cancel(cancelled)?;
    let Some((actor, mut reader)) = country_reader(store, key, branch, tag, cancelled)? else {
        return Ok(None);
    };
    let Some((first, ordinal, event, presence)) = order_at(&mut reader, actor, hour)? else {
        return Ok(None);
    };
    let next = reader
        .events(&[actor], i32::MIN, i32::MAX, ordinal + 1, 1)?
        .rows
        .first()
        .map(|row| row.hour);
    let replay = source.replay_ref();
    let block = replay
        .index
        .iter()
        .find(|block| block.branch == branch && block.first_hour <= hour && hour <= block.last_hour)
        .ok_or_else(|| crate::invalid("Prepared focus hour was not recorded"))?;
    let start = reader
        .latest_event(actor + 2, block.first_hour)?
        .map(|(_, row)| serde_json::from_value::<Progress>(row))
        .transpose()?;
    let progress = progress_at(replay, block, tag, hour, start, cancelled)?.ok_or_else(|| {
        crate::invalid("Prepared focus progress is not observed at this hour; prepare again")
    })?;
    Ok(Some(Hour {
        schema: StateSchema,
        slot: event.slot,
        country_id: event.country_id,
        hour: first.max(presence.first_hour),
        last_hour: next.map_or(presence.last_hour, |next| presence.last_hour.min(next - 1)),
        order_first_hour: Some(event.order_first_hour),
        preparing: Vec::new(),
        current: event.current,
        continuous: event.continuous,
        paused: event.paused,
        completed: pages(ordinal, event.len),
        at: hour,
        progress_raw: progress.raw.to_string(),
        daily_gain_raw: daily_gain(&progress).map(|gain| gain.to_string()),
    }))
}

/// The recorded order and progress, without attributing dates to completions.
pub fn sampled_hour(
    source: &Source,
    branch: u64,
    tag: &str,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<Option<Hour>> {
    use crate::prepared::query::progressive;
    // One focus update period, including the hour before its latest gain.
    let frames = progressive::read_window(
        source,
        branch,
        hour.saturating_sub(replay::cadence::STAGGER_PERIOD),
        hour,
        &READ_TABLES,
        cancelled,
    )?;
    let Some(frame) = frames.last().filter(|frame| frame.hour == hour) else {
        return Ok(None);
    };
    let read = read_hour(frame, Some(tag), &HourRead::default(), false);
    let Some(observation) = read.tags.get(tag) else {
        return Ok(None);
    };
    let mut builder = Builder::new(Some(tag));
    for frame in &frames {
        builder.hour(frame)?;
    }
    let facts = builder.finish();
    let progress = facts
        .tags
        .get(tag)
        .and_then(|stretches| stretches.last())
        .filter(|stretch| stretch.last == hour)
        .map(|stretch| stretch_progress(None, stretch));
    let mut completed = pages(0, observation.order.completed.len as u64);
    completed.sampled = true;
    let supplying = progressive::latest_read(
        source,
        branch,
        hour,
        TableId::Focus,
        replay::cadence::STAGGER_PERIOD,
        &[TableId::Focus],
        cancelled,
    )?;
    let daily_gain_raw = progress
        .as_ref()
        .filter(|progress| {
            supplying
                .as_ref()
                .is_some_and(|read| read.hour >= progress.first)
        })
        .and_then(daily_gain)
        .map(|gain| gain.to_string());
    let mut preparing = vec!["orderFirstHour".into(), "completedByHour".into()];
    if daily_gain_raw.is_none() && observation.order.current.is_some() {
        preparing.push("dailyGainRaw".into());
    }
    Ok(Some(Hour {
        schema: StateSchema,
        slot: observation.slot,
        country_id: observation.country_id,
        hour,
        last_hour: hour,
        order_first_hour: None,
        preparing,
        current: observation.order.current.as_ref().map(Recorded::text),
        continuous: observation.order.continuous.as_ref().map(Recorded::text),
        paused: observation.order.paused,
        completed,
        at: hour,
        progress_raw: observation.progress.to_string(),
        daily_gain_raw,
    }))
}

pub fn sampled_page(
    source: &Source,
    branch: u64,
    tag: &str,
    hour: i32,
    requested: &Pages,
    page: u64,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let Some(frame) = crate::prepared::query::progressive::frame_at(
        source,
        branch,
        hour,
        &READ_TABLES,
        cancelled,
    )?
    else {
        return Ok(serde_json::Value::Null);
    };
    let read = read_hour(&frame, Some(tag), &HourRead::default(), false);
    let Some(observation) = read.tags.get(tag) else {
        return Ok(serde_json::Value::Null);
    };
    let actual = pages(0, observation.order.completed.len as u64);
    if (actual.first, actual.count, actual.rows)
        != (requested.first, requested.count, requested.rows)
    {
        return Err(crate::invalid(
            "Recorded focus page changed. Refresh this view.",
        ));
    }
    if page >= actual.count {
        return Ok(serde_json::Value::Null);
    }
    let first = page
        .checked_mul(PAGE_ROWS as u64)
        .ok_or_else(|| crate::invalid("Focus page index overflow"))?;
    let entries: Vec<_> = observation
        .order
        .completed
        .tokens()
        .into_iter()
        .enumerate()
        .skip(usize::try_from(first).map_err(|_| crate::invalid("Focus page index overflow"))?)
        .take(PAGE_ROWS)
        .map(|(ordinal, token)| FocusEntry {
            ordinal: ordinal as u32,
            token: token.text(),
            completed_by_hour: None,
        })
        .collect();
    Ok(serde_json::to_value(entries)?)
}

/// `tag`'s progress at `hour` of `block`: the block read from its first hour
/// through `hour` alone, its stretches carried on `start` (the progress the
/// block's first hour starts from). None when `tag` is not observed at
/// `hour`. This is the fold's own reading of the block, so it equals what
/// preparation carried there.
pub(crate) fn progress_at(
    replay: &Replay,
    block: &BlockIndex,
    tag: &str,
    hour: i32,
    start: Option<Progress>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Progress>> {
    let mut builder = Builder::new(Some(tag));
    #[cfg(test)]
    BLOCK_READS.with(|reads| reads.borrow_mut().push(block.first_hour));
    replay.read_block_changes(
        block,
        &family::fields()?,
        |change| match change {
            BlockChange::Changed { frame, .. } if frame.hour <= hour => builder.hour(frame),
            BlockChange::Changed { .. } => Ok(()),
            BlockChange::Quiet(quiet) => {
                let hours: Vec<i32> = quiet
                    .hours()
                    .iter()
                    .copied()
                    .filter(|quiet| *quiet <= hour)
                    .collect();
                builder.quiet(&hours)
            }
        },
        || check_cancel(cancelled),
    )?;
    let facts = builder.finish();
    let Some(stretches) = facts.tags.get(tag) else {
        return Ok(None);
    };
    let mut progress = start;
    for stretch in stretches {
        progress = Some(stretch_progress(progress.as_ref(), stretch));
    }
    Ok(progress.filter(|progress| progress.last == hour))
}

// The first hours of the blocks progress was read from, on this thread.
#[cfg(test)]
thread_local! {
    pub(super) static BLOCK_READS: std::cell::RefCell<Vec<i32>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The four focus tables' refusals at `hour`, with the hours they hold.
pub fn query_unavailable(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<Option<UnavailableHour>> {
    let mut reader = Reader::new(store, key, format!("focus-{branch}"), cancelled)?;
    let Some(run) = reader.value_at(0, hour)? else {
        return Ok(None);
    };
    let Some((first, row)) = reader.latest_event(0, hour)? else {
        return Ok(None);
    };
    let reasons: Vec<TableRefusal> = serde_json::from_value(row)?;
    Ok((!reasons.is_empty()).then(|| UnavailableHour {
        schema: StateSchema,
        first_hour: first.max(run.first_hour),
        last_hour: run.last_hour,
        reasons,
    }))
}

/// One page of `tag`'s completed focuses at `hour`, read from its completion
/// chain; `requested` must be the pages `focus-hour` gave for that hour.
#[allow(clippy::too_many_arguments)]
pub fn order_page(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    tag: &str,
    hour: i32,
    requested: &Pages,
    page: u64,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let unavailable = || crate::invalid("Prepared focus data is unavailable at this hour");
    let Some((actor, mut reader)) = country_reader(store, key, branch, tag, cancelled)? else {
        return Err(unavailable());
    };
    let Some((_, ordinal, event, _)) = order_at(&mut reader, actor, hour)? else {
        return Err(unavailable());
    };
    let actual = pages(ordinal, event.len);
    if (actual.first, actual.count, actual.rows)
        != (requested.first, requested.count, requested.rows)
    {
        return Err(crate::invalid(
            "Prepared focus page changed. Refresh this view.",
        ));
    }
    if page >= actual.count {
        return Ok(serde_json::Value::Null);
    }
    let first = page
        .checked_mul(PAGE_ROWS as u64)
        .ok_or_else(|| crate::invalid("Focus page index overflow"))?;
    let rows = (event.len - first).min(PAGE_ROWS as u64);
    let read = reader
        .events(
            &[actor + 1],
            i32::MIN,
            i32::MAX,
            event.chain + first,
            u32::try_from(rows).map_err(|_| crate::invalid("Focus page size overflow"))?,
        )?
        .rows;
    let entries = read
        .into_iter()
        .map(|row| serde_json::from_value::<FocusEntry>(row.row))
        .collect::<Result<Vec<_>, _>>()?;
    check_cancel(cancelled)?;
    if entries.len() as u64 != rows
        || entries
            .iter()
            .enumerate()
            .any(|(index, entry)| u64::from(entry.ordinal) != first + index as u64)
    {
        return Err(crate::invalid("Prepared focus order is incomplete"));
    }
    Ok(serde_json::to_value(entries)?)
}
