//! Replies while air superiority is still being prepared: the prepared
//! prefix answers its hours, and every later recorded hour is pending.
use super::*;
use dashboard_base::prepared::{
    coverage::Coverage,
    prefix::{self, Prefix},
};
use dashboard_base::products::country_groups::Scan as Groups;

pub(super) fn moment(
    source: &Source,
    params: &AirParams,
    context: &LoadContext,
    coverage: Option<Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let branch = params.branch_id.parse().expect("validated branch");
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let prefix = Prefix::new(
        source,
        branch,
        prefix::extent(source, branch, coverage.clone())?,
        params.last_hour,
    );
    let groups = if coverage.is_some() {
        read_groups(&store, &key, branch)?
    } else {
        Groups::new(source.replay_ref(), branch)
    };
    let anchors = assignment::historical_anchors(&groups);
    let mut sides = Vec::new();
    let mut reached = params.clone();
    reached.last_hour = prefix.through(params.last_hour);
    let (first, _) = hours(params)?;
    let samples = if coverage.is_some() && first <= reached.last_hour.unwrap_or(first) {
        Prepared::read_query(&store, &key, branch)?.samples(
            &store,
            &key,
            branch,
            &reached,
            &std::sync::OnceLock::new(),
            &groups,
            &anchors,
            &mut sides,
            cancelled,
        )?
    } else {
        Vec::new()
    };
    let mut reply = AirReply::Moment(moment_reply(samples, sides, &groups, params, &anchors)?);
    resolve_names(
        source,
        &groups,
        context,
        params.last_hour.unwrap_or(first),
        None,
        &mut reply,
    );
    check_reply_size(&reply)?;
    let mut data = serde_json::to_value(reply)?;
    prefix.mark_revisable(source, branch, &mut data)?;
    Ok(data)
}

/// A timeline or the side choices from the prepared prefix, marked with the
/// hours it covers and those still pending; none is estimated.
pub(super) fn query(
    source: &Source,
    params: &AirParams,
    context: &LoadContext,
    coverage: Option<Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let branch = params.branch_id.parse().expect("validated branch");
    let replay = source.replay_ref();
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let after = coverage
        .as_ref()
        .map_or(i32::MIN, |coverage| coverage.last_hour);
    let daily = match coverage {
        Some(_) => super::daily_state(source, &store, &key, branch, cancelled)?,
        None => Arc::new(reader::Daily::empty(Groups::new(replay, branch))),
    };
    let end = params
        .last_hour
        .or_else(|| {
            source
                .block_windows(branch)
                .last()
                .map(|window| window.last_hour)
        })
        .unwrap_or(after);
    let reply = if params.view == AirView::Timeline {
        let reads = reader::Reads {
            source: Some(source),
            store: &store,
            key: &key,
            branch,
            cancelled,
        };
        present_timeline(
            source,
            &daily.groups,
            &daily.anchors,
            context,
            end,
            reader::timeline(&daily, &reads, params, after)?,
            cancelled,
        )?
    } else {
        let mut cache = daily.choices();
        let mut params = params.clone();
        let mut pending = false;
        // The prepared prefix's theatre membership; none is known before it.
        if params
            .theatres
            .as_ref()
            .is_some_and(|theatres| !theatres.is_empty())
        {
            let first = params.first_hour.unwrap_or(i32::MIN);
            let last = params.last_hour.unwrap_or(i32::MAX).min(after);
            if first <= last {
                cache.segments.push(Arc::new(super::theatre_membership(
                    &store, &key, branch, first, last, cancelled,
                )?));
            } else {
                params.theatres = None;
                pending = true;
            }
        }
        let mut reply = cache_reply(
            source,
            replay,
            &daily.groups,
            &cache,
            &params,
            context,
            end,
            cancelled,
        )?;
        if let AirReply::Choices(choices) = &mut reply
            && pending
        {
            choices.regions.clear();
        }
        reply
    };
    let mut data = serde_json::to_value(reply)?;
    Prefix::new(
        source,
        branch,
        prefix::extent(source, branch, coverage)?,
        params.last_hour,
    )
    .mark(&mut data)?;
    data["approximate"] = serde_json::json!([]);
    data["rowsApproximate"] = serde_json::Value::Bool(true);
    dashboard_base::products::check_reply_budget(&data)?;
    Ok(data)
}
