//! The check of a native recording's Campaign rows its preparation makes before anything else:
//! a row naming another campaign than its envelope makes the recording corrupt.

use super::check_cancel;
use dashboard_text::metadata::validate_campaign;
use replay::{
    columnar_frames::{Damage, Recording, Views, Walker, schema},
    campaign::Replay,
    segments::State,
};
use replay_data::rows::TableId;
use std::{io, sync::atomic::AtomicBool};

#[cfg(test)]
mod tests;

/// Refuses `recording` with the row path's refusal of a frame ([`validate_campaign`]) when its
/// Campaign table is complete at an hour and holds anything but `replay`'s campaign row. Its
/// Campaign at an hour is the hour before's but on a page sectioning it or changing its status
/// (from empty tables at its run's start, which are no complete table), so of each segment only
/// its hours through the last such page are walked, Campaign alone; and from its first hour a
/// segment after damage in its run, whose restart may hold what the damage hid. A walk meeting
/// damage stops there, as the run loses those hours. Each segment the recording records is
/// checked once, however its walks read it ([`Recording::recorded_segments`]).
pub(super) fn check(
    recording: &mut Recording,
    replay: &Replay,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    recording.select(&[TableId::Campaign]);
    let recording = &*recording;
    let table = TableId::Campaign.index() as u32;
    let segments = recording.recorded_segments();
    let mut views = Views::new(&[], replay.format, replay.capture_mode)?;
    let mut state = State::new(schema()?)?;
    let mut walker = Walker::new(recording)?;
    let mut lost = false;
    'segments: for (at, segment) in segments.iter().enumerate() {
        check_cancel(cancelled)?;
        let run = recording.run(segment.run)?;
        if segment.restart.is_none() {
            lost = false;
        }
        // A restart ends its page: a segment's pages follow its own through the next one's.
        let end = segments
            .get(at + 1)
            .filter(|next| next.run == segment.run)
            .and_then(|next| next.restart)
            .map_or(run.pages(), |page| page + 1);
        let mut through = std::mem::take(&mut lost).then_some(segment.first);
        for page in segment.restart.map_or(0, |page| page + 1)..end {
            match recording.page(segment.run, page) {
                Ok(page) => {
                    if page.sectioned.get(table) || page.status_changed.get(table) {
                        let last = page.first.saturating_add(page.hours.saturating_sub(1));
                        through = Some(last.min(segment.last));
                    }
                }
                Err(error) if Damage::of(&error).is_some() => {
                    lost = true;
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        let Some(through) = through else {
            continue;
        };
        // A damaged restart is walked past from an earlier one, as the run walks it.
        let mut from = at;
        loop {
            match walker.begin(&segments[from], &mut state) {
                Ok(()) => break,
                Err(error) if Damage::of(&error).is_some() => {
                    if segments[from].restart.is_none() {
                        lost = true;
                        continue 'segments;
                    }
                    from -= 1;
                }
                Err(error) => return Err(error),
            }
        }
        for (index, walked) in (from..=at).zip(&segments[from..=at]) {
            let last = if index == at { through } else { walked.last };
            let read = walker.walk(walked, &mut state, |state, manifest, first| {
                #[cfg(test)]
                tests::WALKED.with(|walked| walked.set(walked.get() + 1));
                validate_campaign(&views.advance(state, manifest, first)?, replay)?;
                let ordinal = i64::from(manifest.hour) - i64::from(run.first_hour);
                Ok(ordinal < i64::from(last))
            });
            match read {
                Ok(_) => {}
                Err(error) if Damage::of(&error).is_some() => {
                    lost = true;
                    continue 'segments;
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}
