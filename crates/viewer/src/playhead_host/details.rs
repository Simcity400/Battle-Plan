//! The detail report's table lines at the playback head: every table's status at the hour the
//! map's mirrors are at, read from the recording's block pages (no table is opened for it), with
//! its live rows where a mirror at the head holds the table, and an unavailable table's reason as
//! the recording words it. A report, not a frame: it waits for a reason's words (bounded, as the
//! proof's limitation comparison waits), never printing that they are being read.

#[cfg(test)]
mod tests;

use super::{Entry, MapSide, limitations, mirror::Mirrors};
use playhead::Timeline;
use replay_columnar::{
    manifest::Status,
    unit::{BLOCK, reader},
};
use replay_data::{Hour, rows::TableId};

/// The lines of the map's reader of the playhead, in the report's words.
pub(crate) fn of_map(side: &MapSide) -> Vec<String> {
    lines(side.entry(), side.applied(), side.parts().map(|(mirrors, _)| mirrors))
}

/// Each table's line at `hour` (none: the head has reached no hour yet), and the capture time's.
pub(crate) fn lines(entry: &Entry, hour: Option<Hour>, mirrors: Option<&Mirrors>) -> Vec<String> {
    let Some(hour) = hour else {
        return vec!["At the playback head: no hour reached yet".to_owned()];
    };
    let timeline = entry.playhead().timeline();
    let mut lines = Vec::with_capacity(TableId::ALL.len() + 2);
    lines.push(format!("At the playback head: hour {hour}"));
    for table in TableId::ALL {
        let held = mirrors.and_then(|mirrors| mirrors.get(table));
        lines.push(match status(entry, timeline, table, hour) {
            Err(error) => format!("{table:?}: its status cannot be read at the head ({error})"),
            Ok(None) => format!("{table:?}: not recorded here: a gap in the recording"),
            Ok(Some((Status::Complete, _))) => match held {
                Some(mirror) => format!("{table:?}: {} rows", mirror.rows().count()),
                None => format!("{table:?}: recorded; its rows are not held at the head"),
            },
            Ok(Some((Status::Truncated { missing }, _))) => format!("{table:?}: truncated, {missing} missing"),
            Ok(Some((Status::Unavailable { reason }, place))) => format!("{table:?}: unavailable ({})", limitations::words(entry, table, reason, place)),
            Ok(Some((Status::NotRecorded, _))) => match entry.capture.get() {
                Some(mode) => format!("{table:?}: not recorded in a {mode} recording"),
                None => format!("{table:?}: not recorded at this hour"),
            },
        });
    }
    lines.push("Capture time: not recorded in the playhead's units".to_owned());
    lines
}

/// What the report says of each table at the head against what today's frame of the same hour
/// says: its status, an unavailable table's reason and its rows where a mirror holds the table.
/// Entries compared, differing, the first difference, and the tables counted apart: not recorded
/// at the head because the unit files hold nothing of them at all.
pub(crate) fn differ(side: &MapSide, today: &replay_frame::frame::DecodedFrame) -> (u64, u64, Option<String>, Vec<TableId>) {
    use replay_frame::frame::TableView;
    let Some(hour) = side.applied() else { return (1, 1, Some("the head has reached no hour".into()), Vec::new()) };
    let (entry, mirrors) = (side.entry(), side.parts().map(|(mirrors, _)| mirrors));
    let (mut entries, mut differing, mut first, mut absent) = (0, 0, None, Vec::new());
    for table in TableId::ALL {
        let held = mirrors.and_then(|mirrors| mirrors.get(table));
        let view = &*today.tables[table.index()];
        let ours = status(entry, entry.playhead().timeline(), table, hour).ok().flatten().map(|(status, _)| status);
        let agree = match (ours, view) {
            (Some(Status::Complete), TableView::Complete(rows)) => held.is_none_or(|mirror| mirror.rows().count() == rows.len()),
            (Some(Status::Truncated { missing }), TableView::Truncated { missing: today, .. }) => missing == *today,
            (Some(Status::Unavailable { .. }), TableView::Unavailable { reason }) => held.and_then(|mirror| mirror.reason()).is_none_or(|text| text == reason),
            (Some(Status::NotRecorded) | None, TableView::NotRecorded { .. }) => true,
            _ => false,
        };
        if !agree && ours == Some(Status::NotRecorded) && !held_at_all(entry, table) {
            absent.push(table);
            continue;
        }
        entries += 1;
        if !agree {
            differing += 1;
            first.get_or_insert_with(|| format!("{table:?} at {hour}: at the head {ours:?} ({:?} rows held), today {}", held.map(|mirror| mirror.rows().count()), match view {
                TableView::Complete(rows) => format!("{} rows", rows.len()),
                TableView::Truncated { missing, .. } => format!("truncated, {missing} missing"),
                TableView::Unavailable { reason } => format!("unavailable ({reason})"),
                TableView::NotRecorded { mode } => format!("not recorded in a {mode} recording"),
            }));
        }
    }
    (entries, differing, first, absent)
}

/// Whether the unit files hold anything of `table`: a part or a restore in some block of some run.
pub(crate) fn held_at_all(entry: &Entry, table: TableId) -> bool {
    entry.pages().runs().iter().flat_map(|run| &run.pages).filter_map(|page| page.tables.get(table.index())).any(|held| !held.parts.is_empty() || held.first.is_some() || held.restore.is_some())
}

/// `table`'s status at game hour `hour` from its block page, with the run and block it is read
/// from; none in a gap between pieces.
pub(super) fn status(entry: &Entry, timeline: &Timeline, table: TableId, hour: Hour) -> std::io::Result<Option<(Status, (usize, usize))>> {
    let Some(piece) = timeline.pieces().iter().find(|piece| piece.first <= hour && hour <= piece.last) else {
        return Ok(None);
    };
    let run = &entry.pages().runs()[piece.run];
    let at = (hour - run.first_hour) as u32;
    let block = at / BLOCK;
    let page = run.pages.get(block as usize).ok_or_else(|| std::io::Error::other(format!("no block {block} in run {}", piece.run)))?;
    let held = page.tables.get(table.index()).ok_or_else(|| std::io::Error::other(format!("block {block} names no table {}", table.index())))?;
    let start = block * BLOCK;
    let (status, _, _, _) = reader::status(held, (at - start) as u16, run.first_hour + start as Hour)?;
    Ok(Some((status, (piece.run, block as usize))))
}
