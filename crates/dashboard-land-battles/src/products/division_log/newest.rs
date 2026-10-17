//! A local census and adjacent drains, seeded with the earlier seen handles.
use super::*;
use dashboard_base::prepared::query::{earlier, progressive};
use replay::container::TableFields;
use std::cell::RefCell;

pub(super) fn query(
    source: &Source,
    params: &DivisionLogParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Option<(DivisionLogReply, bool, bool)>> {
    let branch = branch(source, &params.branch_id)?;
    let (_, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    earlier::cached(
        source,
        "division-log:window",
        branch,
        params.window,
        &key,
        cancelled,
        || query_uncached(source, params, context, cancelled),
    )
}

fn query_uncached(
    source: &Source,
    params: &DivisionLogParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Option<(DivisionLogReply, bool, bool)>> {
    let branch = branch(source, &params.branch_id)?;
    let blocks: Vec<_> = source
        .replay_ref()
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .collect();
    let target = blocks
        .get(params.window)
        .ok_or_else(|| dashboard_core::invalid("Division window not in branch"))?;
    let start = params.window.saturating_sub(1);
    let scan = RefCell::new(Scan::new(source.replay_ref().format));
    // An extra seek before the supplying block is only a numeric sampler's
    // baseline; the census proof starts at the supplying block itself.
    let mut anchored = start == 0;
    let country_anchor = RefCell::new(start == 0);
    let proof = RefCell::new(&mut anchored);
    progressive::state_tail(
        source,
        branch,
        None,
        &TABLES,
        &[],
        &[
            blocks[start].first_hour,
            target.first_hour,
            target.last_hour,
        ],
        &|frame, _| {
            if frame.hour < blocks[start].first_hour || frame.hour > target.last_hour {
                return Ok(Default::default());
            }
            let mut scan = scan.borrow_mut();
            scan.observe_with_changes(frame, None);
            if frame.hour < target.first_hour && frame.complete::<tables::Divisions>().is_ok() {
                **proof.borrow_mut() = true;
            }
            if frame.hour < target.first_hour
                && frame.complete::<tables::Countries>().is_ok()
                && frame.complete::<tables::CountryLinks>().is_ok()
            {
                *country_anchor.borrow_mut() = true;
            }
            Ok(Default::default())
        },
        cancelled,
    )?;
    drop(proof);
    let mut scan = scan.into_inner();
    // The supplying census also needs a dated template read before the
    // window; otherwise its presentation date would be invented.
    if !anchored
        || !country_anchor.into_inner()
        || params.window > 0
            && scan.events.iter().any(|event| {
                event.row.read_hour >= target.first_hour && event.row.template_hour.is_none()
            })
    {
        return Ok(None);
    }
    // Earlier handles only decide whether a creation is a reappearance.
    // Losses and transfers already have their local census/drain proof.
    scan.events
        .retain(|event| event.row.read_hour >= target.first_hour);
    if scan
        .events
        .iter()
        .any(|event| event.row.kind == DivisionLogKind::Created)
    {
        let earlier = earlier::keys(
            source,
            "division-log:keys",
            &blocks[..start],
            &[TableFields::named(TableId::Divisions, &["reveal_skip"])?],
            |frame, facts| {
                if let Ok(rows) = frame.complete::<tables::Divisions>() {
                    facts.available = true;
                    facts.values.extend(
                        rows.iter()
                            .filter(|row| row.reveal_skip == 0)
                            .map(|row| row.id),
                    );
                }
            },
            cancelled,
        )?;
        scan.events.retain(|event| {
            event.row.kind != DivisionLogKind::Created
                || !earlier
                    .values
                    .contains(&event.row.division.parse::<u64>().expect("division handle"))
        });
    }
    Ok(Some((
        scan.reply(source, params, context, cancelled)?,
        scan.available,
        scan.losses_available,
    )))
}
