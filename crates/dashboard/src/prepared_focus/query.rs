//! Focus request adapters over the shared series/event reader.
use super::*;
use crate::prepared::series::Reader;
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
fn pages(first: u64, rows: usize) -> Pages {
    Pages {
        first,
        count: (rows as u64).div_ceil(PAGE_ROWS as u64),
        rows: rows as u64,
    }
}
pub fn query_hour(
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
    let Some(raw) = reader.value_at(actor, hour)? else {
        return Ok(None);
    };
    let gain = reader
        .value_at(actor + 1, hour)?
        .ok_or_else(|| crate::invalid("Prepared focus gain is missing"))?;
    let Some((first, row)) = reader.latest_event(actor, hour)? else {
        return Ok(None);
    };
    let state: FocusState = serde_json::from_value(row)?;
    let count = reader.event_count_through(actor, hour)?;
    let next = reader
        .events(&[actor], i32::MIN, i32::MAX, count, 1)?
        .rows
        .first()
        .map(|row| row.hour);
    Ok(Some(Hour {
        schema: StateSchema,
        slot: state.slot,
        country_id: state.country_id,
        hour: first.max(raw.first_hour).max(gain.first_hour),
        last_hour: next.map_or(raw.last_hour.min(gain.last_hour), |next| {
            raw.last_hour.min(gain.last_hour).min(next - 1)
        }),
        order_first_hour: state.order_first_hour,
        current: state.order.current,
        continuous: state.order.continuous,
        paused: state.order.paused,
        progress_raw: raw.value.to_string(),
        daily_gain_raw: (gain.value != 0).then(|| gain.value.to_string()),
        completed: pages(count - 1, state.order.completed.len()),
        presentation: pages(count - 1, state.presentation.len()),
        details: state
            .details
            .as_ref()
            .map(|items| pages(count - 1, items.len())),
        warnings: state.warnings,
    }))
}
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
#[allow(clippy::too_many_arguments)]
fn read_page<T: serde::de::DeserializeOwned>(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    tag: &str,
    pages: &Pages,
    page: u64,
    field: &str,
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<T>>> {
    check_cancel(cancelled)?;
    if page >= pages.count {
        return Ok(None);
    }
    let Some((actor, mut reader)) = country_reader(store, key, branch, tag, cancelled)? else {
        return Ok(None);
    };
    let event = reader
        .events(&[actor], i32::MIN, i32::MAX, pages.first, 1)?
        .rows
        .pop();
    let Some(event) = event else { return Ok(None) };
    let rows = if field == "completed" {
        &event.row["order"][field]
    } else {
        &event.row[field]
    };
    let rows = rows
        .as_array()
        .ok_or_else(|| crate::invalid("Prepared focus rows are missing"))?;
    let first = usize::try_from(page)
        .ok()
        .and_then(|page| page.checked_mul(PAGE_ROWS))
        .ok_or_else(|| crate::invalid("Focus page index overflow"))?;
    rows.iter()
        .skip(first)
        .take(PAGE_ROWS)
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
        .map_err(Into::into)
}

/// Validate the requested hour and page in the same reader that supplies its rows.
#[allow(clippy::too_many_arguments)]
pub fn query_page(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    tag: &str,
    hour: i32,
    requested: &Pages,
    page: u64,
    product: &str,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let Some((actor, mut reader)) = country_reader(store, key, branch, tag, cancelled)? else {
        return Err(crate::invalid(
            "Prepared focus data is unavailable at this hour",
        ));
    };
    if reader.value_at(actor, hour)?.is_none() {
        return Err(crate::invalid(
            "Prepared focus data is unavailable at this hour",
        ));
    }
    if reader.value_at(actor + 1, hour)?.is_none() {
        return Err(crate::invalid("Prepared focus gain is missing"));
    }
    let Some((_, event)) = reader.latest_event(actor, hour)? else {
        return Err(crate::invalid(
            "Prepared focus data is unavailable at this hour",
        ));
    };
    let ordinal = reader.event_count_through(actor, hour)? - 1;
    let state: FocusState = serde_json::from_value(event)?;
    match product {
        "focus-order-page" => {
            page_value(&state.order.completed, ordinal, requested, page, cancelled)
        }
        "focus-presentation-page" => {
            page_value(&state.presentation, ordinal, requested, page, cancelled)
        }
        "focus-details-page" => page_value(
            &state.details.ok_or_else(|| {
                crate::invalid("Prepared focus description is unavailable at this hour")
            })?,
            ordinal,
            requested,
            page,
            cancelled,
        ),
        _ => Err(crate::invalid("Unknown prepared focus page")),
    }
}
fn page_value<T: serde::Serialize>(
    rows: &[T],
    ordinal: u64,
    requested: &Pages,
    page: u64,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let actual = pages(ordinal, rows.len());
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
    let first = usize::try_from(page)
        .ok()
        .and_then(|page| page.checked_mul(PAGE_ROWS))
        .ok_or_else(|| crate::invalid("Focus page index overflow"))?;
    check_cancel(cancelled)?;
    Ok(serde_json::to_value(
        &rows[first..first.saturating_add(PAGE_ROWS).min(rows.len())],
    )?)
}

pub fn order_page(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    tag: &str,
    pages: &Pages,
    page: u64,
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<FocusEntry>>> {
    read_page(store, key, branch, tag, pages, page, "completed", cancelled)
}
pub fn presentation_page(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    tag: &str,
    pages: &Pages,
    page: u64,
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<PresentedFocus>>> {
    read_page(
        store,
        key,
        branch,
        tag,
        pages,
        page,
        "presentation",
        cancelled,
    )
}
pub fn details_page(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    tag: &str,
    pages: &Pages,
    page: u64,
    cancelled: &AtomicBool,
) -> io::Result<Option<Vec<PresentedFocus>>> {
    read_page(store, key, branch, tag, pages, page, "details", cancelled)
}
pub fn static_text_page(
    store: &store::Store,
    key: &store::SourceKey,
    tokens: &[String],
    cancelled: &AtomicBool,
) -> io::Result<Vec<Artwork>> {
    check_cancel(cancelled)?;
    let sections: Vec<_> = tokens
        .iter()
        .map(|token| static_text_section(token))
        .collect();
    let ranges: Vec<_> = sections
        .iter()
        .map(|section| (section.as_str(), 0))
        .collect();
    store
        .read_many(key, &ranges)?
        .into_iter()
        .zip(tokens)
        .map(|(file, token)| {
            check_cancel(cancelled)?;
            let file = file.ok_or_else(|| {
                crate::invalid(format!("Prepared focus static text unavailable: {token}"))
            })?;
            Ok(serde_json::from_reader(file)?)
        })
        .collect()
}
