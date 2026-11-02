//! The completions the event feed lists, read from the row recording frame by frame as the
//! native preparation reads them from its walk ([`super::facts`], [`super::emit`]): every
//! country observed at every complete read with its presentations resolved anew. The parity
//! oracle of the family's feed.

use super::{
    Naming, ProjectFacts, history, history_operand,
    emit::{self, Completions},
    facts::{self, Binding, Observed, Project},
    query::Feed,
};
use dashboard_base::{Source, products::LoadContext};
use dashboard_text::metadata::{check_cancel, slot_read, validate_campaign};
use replay_data::rows::{ProjectHistoryRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, FrameDecoder};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// What the observation of `frame` changes of the completions.
fn observe(
    frame: &DecodedFrame,
    start: i32,
    content: &mut Option<super::Content>,
    (completions, unrecorded): (&mut Completions, &dyn Fn(i32) -> bool),
) -> Vec<emit::Change> {
    let mut changes = Vec::new();
    let Ok(projects) = frame.complete::<tables::Projects>() else {
        return changes;
    };
    let actors = history::Actors::read(frame);
    let mut histories = BTreeMap::<(u32, u32), Vec<&ProjectHistoryRow>>::new();
    let history = frame.complete::<tables::ProjectHistory>().ok();
    for row in history.into_iter().flatten() {
        histories.entry((row.owner, row.project)).or_default().push(row);
    }
    let mut naming = Naming::default();
    for (tag, &(country_id, slot)) in &actors.tags {
        let Ok(tag) = std::str::from_utf8(tag) else {
            continue;
        };
        let links = actors.links.get(&slot).map_or(&[][..], Vec::as_slice);
        let mut observed = Vec::new();
        for link in links {
            let first = projects.partition_point(|row| row.owner < link.id);
            let count = projects[first..].partition_point(|row| row.owner == link.id);
            for row in &projects[first..first + count] {
                let operands: Vec<(u32, bool, i32)> = links
                    .iter()
                    .flat_map(|owner| histories.get(&(owner.id, row.local_id)))
                    .flatten()
                    .map(|row| history_operand(row))
                    .collect();
                let mut facts = ProjectFacts::project(row, None, frame.hour, start);
                facts.observe_history(
                    history.map(|_| operands.as_slice()),
                    frame.hour,
                    start,
                );
                let progress = std::mem::take(&mut facts.progress);
                let presented = facts::presentation_row(&facts, row.program_reference, link.id);
                let look = naming.look(
                    frame,
                    tag,
                    slot,
                    &presented,
                    facts.iterations,
                    content.as_mut(),
                );
                observed.push(Project {
                    facts,
                    look,
                    progress,
                });
            }
        }
        let observed = Observed {
            tag: Arc::from(tag),
            identity: (slot, country_id),
            seen: facts::Seen::Whole(Some(observed)),
        };
        let facts::Seen::Whole(Some(projects)) = &observed.seen else {
            unreachable!("an observation of its projects")
        };
        completions.observe(
            frame.hour,
            &observed,
            projects,
            content.as_mut(),
            unrecorded,
            &mut changes,
        );
    }
    emit::order(&mut changes);
    changes
}

/// Who holds each holder from `hour`, as the countries of `frame` resolve them, in `completions`:
/// `held`, what they resolved the hour before, `adjacent` when that hour was walked.
fn hold(
    frame: &DecodedFrame,
    adjacent: bool,
    held: &mut BTreeMap<Arc<str>, (u32, u32)>,
    completions: &mut Completions,
) {
    let hour = frame.hour;
    if !adjacent {
        held.clear();
    }
    let mut now = BTreeMap::new();
    for (tag, &(id, slot)) in &history::Actors::read(frame).tags {
        if let Ok(tag) = std::str::from_utf8(tag) {
            now.insert(Arc::<str>::from(tag), (slot, id));
        }
    }
    for (tag, &identity) in &now {
        match held.get(tag) {
            Some(&old) if old == identity => {}
            Some(&old) => {
                completions.release(old, tag, hour - 1);
                completions.hold(identity, tag, hour);
            }
            None => completions.hold(identity, tag, hour),
        }
    }
    for (tag, &old) in held.iter() {
        if !now.contains_key(tag) {
            completions.release(old, tag, hour - 1);
        }
    }
    *held = now;
}

/// The feed of `branch` of the row recording `source` holds, its projects named with the
/// content `context` binds.
pub fn feed(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Feed> {
    let replay = source.replay_ref();
    let binding = Binding::load(source, context)?;
    let bound = binding.emitted();
    let mut content = binding.content().cloned();
    let mut blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .collect();
    blocks.sort_unstable_by_key(|block| block.first_hour);
    let start = blocks.first().map_or(0, |block| block.first_hour);
    let gaps: Vec<(i32, i32)> = blocks
        .windows(2)
        .filter_map(|pair| {
            let first = pair[0].last_hour.checked_add(1)?;
            (first < pair[1].first_hour).then(|| (first, pair[1].first_hour - 1))
        })
        .collect();
    let unrecorded = |hour: i32| emit::unrecorded(start, &gaps, hour);
    let selected = facts::tables(binding.scoped()).to_vec();
    let mut completions = Completions::default();
    let mut names = history::NameScan::default();
    let mut sent = std::collections::HashMap::new();
    let mut held = BTreeMap::new();
    let mut last: Option<i32> = None;
    let mut changes = Vec::new();
    let mut available = false;
    dashboard_core::blocks::scan_blocks(
        replay,
        blocks.into_iter(),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                check_cancel(cancelled)?;
                let Ok(frames) = frames else {
                    continue;
                };
                let mut decoder = FrameDecoder::for_tables(&selected);
                for frame in frames {
                    check_cancel(cancelled)?;
                    let Ok(frame) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(frame, replay).is_err() {
                        break;
                    }
                    let adjacent = last.is_some_and(|last| last + 1 == frame.hour);
                    if !adjacent && let Some(last) = last {
                        completions.close(last);
                    }
                    last = Some(frame.hour);
                    hold(frame, adjacent, &mut held, &mut completions);
                    let spellings = frame
                        .complete::<tables::LexicalTokens>()
                        .map(|rows| {
                            rows.iter()
                                .map(|row| (row.token, String::from_utf8(row.literal.clone()).ok()))
                                .collect()
                        })
                        .unwrap_or_default();
                    completions.tokens(frame.hour, spellings, true);
                    let read =
                        slot_read(replay.format, &frame.metrics, TableId::Projects, frame.hour);
                    if binding.scoped()
                        && let Some(content) = content.as_mut()
                    {
                        names.observe(
                            frame,
                            &history::Actors::read(frame),
                            read,
                            content,
                            history::Reuse::Every,
                            cancelled,
                        )?;
                        let current = names.take_current();
                        let changes = current.changes(&mut sent, &Default::default());
                        completions.names(frame.hour, changes, false);
                        content.scoped.clear();
                        content.scoped.push((branch, Arc::new(current)));
                    }
                    if !read || frame.complete::<tables::Projects>().is_err() {
                        continue;
                    }
                    available |= history::Actors::read(frame)
                        .tags
                        .keys()
                        .any(|tag| std::str::from_utf8(tag).is_ok());
                    changes.extend(observe(
                        frame,
                        start,
                        &mut content,
                        (&mut completions, &unrecorded),
                    ));
                }
            }
            Ok(())
        },
    )?;
    Ok(Feed::of(
        changes,
        available,
        bound.completion_warnings,
        bound.error,
    ))
}
