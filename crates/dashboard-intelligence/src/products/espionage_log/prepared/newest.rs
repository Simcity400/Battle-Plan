//! A newest day anchored at witnessed operation episode boundaries.
use super::*;
use dashboard_base::prepared::query::earlier;
use replay::container::TableFields;

pub(super) fn reply(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Option<EspionageLogReply>> {
    let (_, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    earlier::cached(
        source,
        "espionage-log:window",
        branch,
        source.block_windows(branch).len().saturating_sub(1),
        &key,
        cancelled,
        || reply_uncached(source, branch, context, cancelled),
    )
}

fn reply_uncached(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Option<EspionageLogReply>> {
    let replay = source.replay_ref();
    let blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .collect();
    let Some(target) = blocks.last() else {
        return Ok(None);
    };
    let first = source
        .block_windows(branch)
        .first()
        .expect("branch windows")
        .first_hour;
    let day_first = metadata_windows(first, target.last_hour)
        .last()
        .expect("newest day")
        .first_hour;
    let start = blocks
        .partition_point(|block| block.last_hour < day_first)
        .saturating_sub(1);
    let prior = earlier::keys(
        source,
        "espionage-log:keys",
        &blocks[..start],
        &[
            TableFields::keys(TableId::Operations),
            TableFields::named(TableId::OperationCompletions, &["id"])?,
        ],
        |frame, facts| {
            if let Ok(rows) = frame.complete::<tables::Operations>() {
                facts.values.extend(rows.iter().map(|row| row.id));
            }
            if let Ok(rows) = frame.complete::<tables::OperationCompletions>() {
                facts.values.extend(rows.iter().map(|row| row.id));
            }
        },
        cancelled,
    )?;
    let mut builder = Builder::new(replay);
    let mut countries = super::super::super::operative_rows::CountryInputs::default();
    let mut first_census = None;
    let mut country_anchor = start == 0;
    let mut first_frame = true;
    super::super::super::intelligence_prepared::first_views::visit(
        source,
        branch,
        day_first,
        target.last_hour,
        &[
            TableId::Campaign,
            TableId::Countries,
            TableId::CountryLinks,
            TableId::Operations,
            TableId::OperationCompletions,
            TableId::OperationAssignments,
            TableId::OperativeMemberships,
            TableId::LexicalTokens,
        ],
        cancelled,
        |frame| {
            if frame.hour < blocks[start].first_hour {
                return Ok(());
            }
            countries.update(frame, None);
            if first_frame {
                country_anchor |= frame.complete::<tables::Countries>().is_ok()
                    && frame.complete::<tables::CountryLinks>().is_ok();
                first_frame = false;
            }
            if operations_read(frame) && frame.complete::<tables::Operations>().is_ok() {
                first_census.get_or_insert(frame.hour);
            }
            builder.observe_countries(frame, &countries);
            builder.observe_completions_countries(frame, replay, branch, &countries)
        },
    )?;
    if !country_anchor || start > 0 && first_census.is_none_or(|hour| hour > day_first) {
        return Ok(None);
    }
    let raw = builder.reply(
        branch.to_string(),
        first,
        target.last_hour,
        replay.format.version(),
    );
    let index = metadata_windows(first, target.last_hour).len() - 1;
    let mut reply = metadata_window(&raw, Some(index))?;
    // A later local episode has its absence, finish or changed fingerprint
    // witnessed here. An inherited episode, or an unmatched late completion,
    // still needs its earlier history.
    if reply.operations.iter().any(|operation| {
        operation
            .id
            .split(':')
            .next()
            .and_then(|id| id.parse::<u64>().ok())
            .is_some_and(|id| prior.values.contains(&id))
            && (operation.id.contains(":completion:")
                || first_census.is_none_or(|first| operation.first_read <= first))
    }) {
        return Ok(None);
    }
    present(source, context, cancelled, &mut reply)?;
    Ok(Some(reply))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::espionage_log::tests::frame;
    use dashboard_base::test_fixtures::{continuation_records, file_source, replay};

    #[test]
    fn newest_day_matches_hourly_operations_and_refuses_handles_needing_earlier_episodes() {
        for mode in [0, 1, 2] {
            let mut recorded = replay(vec![
                frame(100, u32::from(mode > 0)),
                frame(101, u32::from(mode == 2)),
            ]);
            for block in 1..14 {
                let hour = 100 + block * 2;
                recorded = recorded
                    .append(&[continuation_records(
                        &recorded,
                        1,
                        vec![
                            frame(hour, u32::from(mode == 2)),
                            frame(hour + 1, u32::from(mode == 2 || block == 13)),
                        ],
                    )])
                    .unwrap();
            }
            let (source, root) = file_source(recorded);
            let context = LoadContext { content: None };
            let cancelled = AtomicBool::new(false);
            let result = reply(&source, 1, &context, &cancelled).unwrap();
            if mode == 2 {
                assert!(
                    result.is_none(),
                    "the inherited episode needs its original first read"
                );
            } else {
                let actual = result.unwrap();
                let raw = crate::products::espionage_log::oracle::scan(
                    source.replay_ref(),
                    &BranchParams {
                        branch_id: "1".into(),
                    },
                    &cancelled,
                )
                .unwrap();
                let expected = metadata_window(&raw, actual.window).unwrap();
                assert_eq!(
                    serde_json::to_value(actual.operations).unwrap(),
                    serde_json::to_value(expected.operations).unwrap()
                );
                let before = dashboard_base::prepared::query::frames::reads(&source);
                let (_, refused) =
                    dashboard_base::prepared::query::progressive::without_block_decodes(|| {
                        reply(&source, 1, &context, &cancelled).unwrap()
                    });
                assert!(!refused);
                assert_eq!(
                    dashboard_base::prepared::query::frames::reads(&source),
                    before
                );
            }
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
