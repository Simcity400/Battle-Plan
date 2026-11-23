#[cfg(test)]
mod tests;

use super::{Entry, MapSide, mirror::Mirrors};
pub mod reasons;
use playhead::Timeline;
use replay_columnar::{
    manifest::Status,
    unit::{BLOCK, reader},
};
use replay_data::{Hour, rows::TableId};

pub fn of_map(side: &MapSide<impl crate::playhead_host::PlaybackProducts>) -> Vec<String> {
    lines(side.entry(), side.applied(), side.parts().map(|(mirrors, _)| mirrors))
}

pub fn lines(entry: &Entry, hour: Option<Hour>, mirrors: Option<&Mirrors>) -> Vec<String> {
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
            Ok(Some((Status::Unavailable { reason }, place))) => format!("{table:?}: unavailable ({})", reasons::words(entry, table, reason, place, std::time::Duration::from_secs(60))),
            Ok(Some((Status::NotRecorded, _))) => match entry.capture.get() {
                Some(mode) => format!("{table:?}: not recorded in a {mode} recording"),
                None => format!("{table:?}: not recorded at this hour"),
            },
        });
    }
    lines.push("Capture time: not recorded in the playhead's units".to_owned());
    lines
}

pub fn differ(side: &MapSide<impl crate::playhead_host::PlaybackProducts>, today: &replay_frame::frame::DecodedFrame) -> (u64, u64, Option<String>, Vec<TableId>) {
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

pub fn held_at_all(entry: &Entry, table: TableId) -> bool {
    entry.pages().runs().iter().flat_map(|run| &run.pages).filter_map(|page| page.tables.get(table.index())).any(|held| !held.parts.is_empty() || held.first.is_some() || held.restore.is_some())
}

pub fn status(entry: &Entry, timeline: &Timeline, table: TableId, hour: Hour) -> std::io::Result<Option<(Status, (usize, usize))>> {
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
