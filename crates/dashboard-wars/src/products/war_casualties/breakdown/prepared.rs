//! The existing breakdown projection over shared casualty windows and war changes.
use super::*;
use dashboard_base::prepared::series::{self, Reader};

pub fn query(
    source: &Source,
    params: &CasualtyBreakdownParams,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Option<CasualtyBreakdownReply>> {
    CasualtyBreakdown::validate(params).map_err(dashboard_core::invalid)?;
    check_cancel(cancelled)?;
    let branch = params.branch_id.parse().expect("validated branch");
    let branch_info = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| dashboard_core::invalid("Unknown casualty branch"))?;
    let bounds = dashboard_text::metadata::branch_bounds(source.replay_ref(), branch_info);
    let windows = source.block_windows(branch).to_vec();
    if params.window.is_some_and(|window| window >= windows.len()) {
        return Err(dashboard_core::invalid("Unknown casualty window"));
    }
    let Some((_, catalogue)) = series::read_catalogue::<super::super::prepared::Catalogue>(
        store, key, "casualty", branch,
    )?
    else {
        return Ok(None);
    };
    let selected = catalogue
        .holders
        .get(&params.tag)
        .cloned()
        .unwrap_or_default();
    let cat = context
        .content
        .as_ref()
        .map(|options| dashboard_base::catalogue::catalogue(source, options))
        .transpose()?;
    let Some(wars) = crate::products::wars::prepared::projection(
        store,
        key,
        source,
        &WarsParams {
            branch_id: params.branch_id.clone(),
            viewpoint_tag: Some(params.tag.clone()),
        },
        Some(&selected),
        &BTreeSet::new(),
        cat.as_deref(),
        cancelled,
    )?
    else {
        return Ok(None);
    };
    let mut coverage = Reader::new(store, key, format!("casualty-coverage-{branch}"), cancelled)?;
    let observed = observed(&mut coverage, bounds)?;
    let projection = if let Some(ordinal) = params.window {
        let countries = countries(&catalogue.holder_coverage, &selected, cancelled)?;
        let Some(window) = series::read_reply::<pairs::Window>(
            store,
            key,
            &format!("casualty-pairs-{branch}"),
            ordinal as u64,
        )?
        else {
            return Ok(None);
        };
        Some(project(
            window
                .records
                .iter()
                .map(|(key, runs)| (key, runs.as_slice())),
            &wars,
            &selected,
            cancelled,
            Some(countries),
        )?)
    } else {
        None
    };
    build_reply(
        params,
        windows,
        &observed,
        catalogue.incomplete,
        &wars,
        projection.as_ref(),
        cancelled,
    )
    .map(Some)
}

/// Whole-branch holder and uncertainty changes retain supplying boundaries
/// and refusals which precede the requested window.
fn countries(
    coverage: &BTreeMap<(Identity, Identity), Vec<pairs::HolderRun>>,
    selected: &HashSet<Identity>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<CasualtyBreakdownCountryRun>> {
    let mut changes = BTreeMap::<i64, Vec<(Identity, bool, i32)>>::new();
    for (&(first, second), spans) in coverage {
        let (identity, endpoint) = if selected.contains(&first) {
            (first, 0)
        } else if selected.contains(&second) {
            (second, 1)
        } else {
            continue;
        };
        for span in spans.iter().filter(|span| span.live[endpoint]) {
            changes
                .entry(i64::from(span.first_hour))
                .or_default()
                .push((identity, span.incomplete, 1));
            changes
                .entry(i64::from(span.last_hour) + 1)
                .or_default()
                .push((identity, span.incomplete, -1));
        }
    }
    country_runs(changes, cancelled)
}

/// Walk canonical coverage boundaries, skipping unchanged publication batches.
fn observed(reader: &mut Reader<'_>, (first, last): (i32, i32)) -> io::Result<Vec<HourSpan>> {
    let mut result = Vec::new();
    let mut hour = i64::from(first);
    while hour <= i64::from(last) {
        let before = hour;
        if let Some(run) = reader.value_at(0, hour as i32)? {
            result.push(HourSpan {
                first_hour: hour as i32,
                last_hour: run.last_hour.min(last),
            });
            hour = i64::from(run.last_hour) + 1;
        } else if let Some(next) = reader.next_change(0, hour as f64, true)? {
            hour = i64::from(next);
        } else {
            break;
        }
        if hour <= before {
            return Err(dashboard_core::invalid(
                "Prepared casualty coverage made no progress",
            ));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
