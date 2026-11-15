use super::*;

/// The tables the line output self-check decodes along the branch.
pub(super) const LINE_OUTPUT_TABLES: [TableId; 2] =
    [TableId::ProductionLineOutput, TableId::ProductionLines];

/// The save keeps no per-line count, so ProductionLineOutput is self-checked
/// along the branch, from its observation start to the compared hour: each
/// publication's live output rows pair with the same read's ProductionLines
/// census (`census::CensusCheck`). Each row's producing country is compared
/// with the save separately.
pub fn compare_line_output(
    replay: &Replay,
    frame: &DecodedFrame,
    report: &mut Report,
) -> io::Result<()> {
    report.limitation(
        "the save keeps no per-line spending or delivered count; the branch self-check pairs each publication's live output rows with the same read's ProductionLines census, while current cost compares with the save separately",
        "ProductionLineOutput",
    );
    report.limitation(
        "country_reports.equipment_production also counts refits and missiles, and a type of several categories in each, so it is at most an approximate check of the line totals and is not compared",
        "ProductionLineOutput",
    );
    let rows = complete::<tables::ProductionLineOutput>(frame)?;
    // Every row of a branch shares its observation start. After the last
    // line ended and its final rows were dropped, the table is empty, but
    // the earlier publications still hold credits the history must match.
    let since = match rows.first() {
        Some(row) => Some(row.since_hour),
        None => observation_start(replay, frame)?,
    };
    let Some(since) = since else {
        report.limitation(
            "no production line output was published on this branch up to this hour",
            "ProductionLineOutput",
        );
        return Ok(());
    };
    if let Some(gap) = replay.gaps.iter().find(|gap| {
        gap.branch == frame.branch && gap.first_hour <= frame.hour && gap.last_hour >= since
    }) {
        report.limitation(
            "a gap since the observation began may hide an ended-line publication; the line output census is not self-checked across it",
            format!(
                "ProductionLineOutput: hours {}..={}: {}",
                gap.first_hour, gap.last_hour, gap.reason
            ),
        );
        return Ok(());
    }
    let mut blocks = replay
        .index
        .iter()
        .filter(|block| {
            block.branch == frame.branch
                && block.first_hour <= frame.hour
                && block.last_hour >= since
        })
        .collect::<Vec<_>>();
    blocks.sort_by_key(|block| block.first_hour);
    let mut census = census::CensusCheck::new(since);
    for block in blocks {
        // Every block starts with a keyframe.
        replay.visit_blocks(
            &[block],
            &LINE_OUTPUT_TABLES,
            |_, _, hour_frame| {
            if hour_frame.hour > frame.hour {
                return Ok(false);
            }
            let decoded = Ok::<&replay_frame::frame::DecodedFrame, std::io::Error>(hour_frame)?;
            if decoded.hour >= since {
                census.observe(decoded, report)?;
            }
                Ok(true)
            },
            || Ok(()),
        )?;
    }
    census.finish(report);
    Ok(())
}

/// The observation start of the branch's latest publication with rows at or
/// before the frame's hour, searching its blocks from the latest back; `None`
/// when no publication up to that hour held a row.
fn observation_start(replay: &Replay, frame: &DecodedFrame) -> io::Result<Option<Hour>> {
    let mut blocks = replay
        .index
        .iter()
        .filter(|block| block.branch == frame.branch && block.first_hour <= frame.hour)
        .collect::<Vec<_>>();
    blocks.sort_by_key(|block| std::cmp::Reverse(block.first_hour));
    for block in blocks {
        let mut since = None;
        replay.visit_blocks(
            &[block],
            &LINE_OUTPUT_TABLES,
            |_, _, hour_frame| {
            if hour_frame.hour > frame.hour {
                return Ok(false);
            }
            let decoded = Ok::<&replay_frame::frame::DecodedFrame, std::io::Error>(hour_frame)?;
            if let Some(row) = complete::<tables::ProductionLineOutput>(decoded)
                .ok()
                .and_then(|rows| rows.first())
            {
                since = Some(row.since_hour);
            }
                Ok(true)
            },
            || Ok(()),
        )?;
        if since.is_some() {
            return Ok(since);
        }
    }
    Ok(None)
}

mod census;
