use super::{
    EventFeedKind,
    index::{Feed, kind_code},
};
use dashboard_query::log::occurrence::{DatedRows, Direction, PlacedGap};
use std::{borrow::Cow, cmp::Reverse, fmt::Write};

pub(crate) const MIN_RUN: usize = 5;
pub(crate) const CHILDREN_SHOWN: usize = 32;
const SCRIPT_GROUP: u16 = 0;
const DECISION_GROUP: u16 = 1;
const KIND_OFFSET: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FoldMeta {
    pub day: i32,
    pub country: u32,
    pub group: u16,
}

pub(crate) fn group_of(kind: EventFeedKind) -> u16 {
    use EventFeedKind::*;
    match kind {
        ScriptEventFired | NewsEventFired | EventOptionSelected => SCRIPT_GROUP,
        DecisionSelected | DecisionRemoved | DecisionCancelled | DecisionTimedOut | DecisionActivated | DecisionCompleted | DecisionReenabled | DecisionRemovalEffect | DecisionCancellationEffect
        | DecisionCompletionEffect | DecisionTimeoutEffect | DecisionDurationChanged => DECISION_GROUP,
        _ => KIND_OFFSET + kind_code(kind),
    }
}

fn group_label(group: u16) -> Cow<'static, str> {
    match group {
        SCRIPT_GROUP => Cow::Borrowed("script-event"),
        DECISION_GROUP => Cow::Borrowed("decision"),
        code => EventFeedKind::of_index(code - KIND_OFFSET)
            .and_then(|kind| serde_json::to_string(&kind).ok())
            .map_or_else(|| Cow::Owned(code.to_string()), |quoted| Cow::Owned(quoted.trim_matches('"').to_owned())),
    }
}

fn clean_tag(tag: &str) -> Cow<'_, str> {
    if tag.bytes().all(|b| !b.is_ascii_whitespace() && b != b':' && b != b'%') {
        return Cow::Borrowed(tag);
    }
    let mut out = String::with_capacity(tag.len());
    for b in tag.bytes() {
        if b.is_ascii_whitespace() || b == b':' || b == b'%' { let _ = write!(out, "%{b:02x}"); } else { out.push(b as char); }
    }
    Cow::Owned(out)
}

pub(crate) fn key_of(meta: FoldMeta, country: &str, anchor: u128) -> String {
    format!("fold:{}:{}:{}:{anchor:x}", meta.day, clean_tag(country), group_label(meta.group))
}

#[derive(Clone, Copy, Debug)]
struct Run {
    offset: u32,
    len: u32,
    newest_date: i32,
    max_observed: i32,
    anchor: u128,
    meta: Option<FoldMeta>,
    lines_before: u64,
}

impl Run {
    fn is_fold(self) -> bool { self.meta.is_some() && self.len as usize >= MIN_RUN }
    fn collapsed(self) -> u64 { if self.is_fold() { 1 } else { u64::from(self.len) } }
}

fn has_gap(marks: &[i32], lo: i32, hi: i32, desc: bool) -> bool {
    if lo >= hi || marks.is_empty() { return false; }
    if desc {
        marks.partition_point(|&m| m < lo) < marks.partition_point(|&m| m < hi)
    } else {
        marks.partition_point(|&m| m <= lo) < marks.partition_point(|&m| m <= hi)
    }
}

pub(crate) enum PlannedLine {
    Event { at: u32, parent: Option<String> },
    Fold { key: String, first_at: u32, observed: i32, count: u32, kind: EventFeedKind, country: String },
    More { key: String, parent: String, hour: i32, total: u32, remaining: u32 },
}

pub(crate) struct FoldedPage {
    pub total: u64,
    pub offset: u64,
    pub lines: Vec<PlannedLine>,
    pub gaps: Vec<PlacedGap>,
}

pub(crate) fn page(feed: &Feed, dated: DatedRows<'_>, direction: Direction, offset: u64, limit: u32, expanded: Option<&[String]>, full: Option<&[String]>) -> FoldedPage {
    let (desc, ordered) = (direction == Direction::Descending, dated.rows);
    let mut gap_marks: Vec<i32> = dated.gaps.iter().map(|g| if desc { g.last_hour } else { g.first_hour }).collect();
    gap_marks.sort_unstable();
    gap_marks.dedup();
    let wanted: Vec<(u128, &str, bool)> = expanded
        .into_iter()
        .flatten()
        .filter_map(|key| {
            let anchor = key.rsplit_once(':').and_then(|(_, hex)| u128::from_str_radix(hex, 16).ok())?;
            Some((anchor, key.as_str(), full.is_some_and(|list| list.iter().any(|item| item == key))))
        })
        .collect();
    let (mut runs, mut candidates, mut count) = (Vec::<Run>::new(), Vec::<usize>::new(), 0u32);
    let mut push = |at: u32| {
        let (occ, meta, run_offset) = (feed.index.get(at), feed.fold_meta(at), count);
        count += 1;
        if let Some(last) = runs.last_mut() && meta.is_some() && last.meta == meta && !has_gap(&gap_marks, last.newest_date, occ.date, desc) {
            let was_fold = last.is_fold();
            last.len += 1;
            last.newest_date = occ.date;
            last.max_observed = last.max_observed.max(occ.observed);
            if !was_fold && last.is_fold() && wanted.iter().any(|w| w.0 == last.anchor) {
                candidates.push(runs.len() - 1);
            }
            return;
        }
        let lines_before = runs.last().map_or(0, |r| r.lines_before + r.collapsed());
        runs.push(Run { offset: run_offset, len: 1, newest_date: occ.date, max_observed: occ.observed, anchor: occ.id, meta, lines_before });
    };
    if desc { for &at in ordered.iter().rev() { push(at); } } else { for &at in ordered { push(at); } }
    let open: Vec<(usize, String, bool, u64)> = candidates
        .into_iter()
        .filter_map(|idx| {
            let run = runs[idx];
            let meta = run.meta?;
            let expected = key_of(meta, feed.country(meta.country), run.anchor);
            let &(_, _, is_full) = wanted.iter().find(|w| w.0 == run.anchor && w.1 == expected)?;
            let len = run.len as usize;
            let extra = (if is_full { len } else { len.min(CHILDREN_SHOWN) }) as u64 + u64::from(len > CHILDREN_SHOWN);
            Some((idx, expected, is_full, extra))
        })
        .collect();
    let total_collapsed = runs.last().map_or(0, |r| r.lines_before + r.collapsed());
    let total = total_collapsed + open.iter().map(|item| item.3).sum::<u64>();
    let clamped = offset.min(total);
    let start_ordinal = |idx: usize| -> u64 {
        let run = runs[idx];
        if desc {
            let at = open.partition_point(|item| item.0 <= idx);
            total_collapsed - (run.lines_before + run.collapsed()) + open[at..].iter().map(|item| item.3).sum::<u64>()
        } else {
            let at = open.partition_point(|item| item.0 < idx);
            run.lines_before + open[..at].iter().map(|item| item.3).sum::<u64>()
        }
    };
    let (mut lines, cap) = (Vec::new(), limit as usize);
    if clamped < total && cap > 0 && !runs.is_empty() {
        let (mut lo, mut hi) = (0usize, runs.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if (desc && start_ordinal(mid) > clamped) || (!desc && start_ordinal(mid) <= clamped) { lo = mid + 1; } else { hi = mid; }
        }
        let (mut cur, mut ordinal) = (if desc { lo } else { lo.saturating_sub(1) }, clamped);
        loop {
            let run = runs[cur];
            let local = (ordinal - start_ordinal(cur)) as usize;
            let members = if desc { &ordered[ordered.len() - run.offset as usize - run.len as usize..ordered.len() - run.offset as usize] } else { &ordered[run.offset as usize..run.offset as usize + run.len as usize] };
            if !run.is_fold() {
                for &at in members[local..].iter().take(cap - lines.len()) {
                    lines.push(PlannedLine::Event { at, parent: None });
                    ordinal += 1;
                }
            } else {
                if local == 0 && lines.len() < cap {
                    let meta = run.meta.unwrap();
                    let country = feed.country(meta.country);
                    let first_at = members[0];
                    let kind = EventFeedKind::of_index(feed.index.get(first_at).kind).unwrap_or(EventFeedKind::ScriptEventFired);
                    lines.push(PlannedLine::Fold { key: key_of(meta, country, run.anchor), first_at, observed: run.max_observed, count: run.len, kind, country: country.to_owned() });
                    ordinal += 1;
                }
                if let Ok(pos) = open.binary_search_by_key(&cur, |item| item.0) {
                    let (_, key, is_full, _) = &open[pos];
                    let visible = if *is_full || members.len() <= CHILDREN_SHOWN {
                        members
                    } else if desc {
                        &members[..CHILDREN_SHOWN]
                    } else {
                        &members[members.len() - CHILDREN_SHOWN..]
                    };
                    for &at in visible[local.saturating_sub(1).min(visible.len())..].iter().take(cap - lines.len()) {
                        lines.push(PlannedLine::Event { at, parent: Some(key.clone()) });
                        ordinal += 1;
                    }
                    if members.len() > CHILDREN_SHOWN && local <= 1 + visible.len() && lines.len() < cap {
                        let remaining = if *is_full { 0 } else { (members.len() - CHILDREN_SHOWN) as u32 };
                        lines.push(PlannedLine::More { key: format!("more:{key}"), parent: key.clone(), hour: feed.index.get(members[0]).date, total: run.len, remaining });
                        ordinal += 1;
                    }
                }
            }
            if lines.len() >= cap || (desc && cur == 0) || (!desc && cur + 1 >= runs.len()) { break; }
            cur = if desc { cur - 1 } else { cur + 1 };
        }
    }
    let mut placed: Vec<(u64, PlacedGap)> = dated
        .gaps
        .iter()
        .map(|gap| {
            let before = if runs.is_empty() {
                0
            } else if desc {
                let first_above = runs.partition_point(|r| r.newest_date <= gap.last_hour);
                if first_above == 0 { total } else { start_ordinal(first_above - 1) }
            } else {
                let first_at = runs.partition_point(|r| r.newest_date < gap.first_hour);
                if first_at >= runs.len() { total } else { start_ordinal(first_at) }
            };
            (before, PlacedGap { first_hour: gap.first_hour, last_hour: gap.last_hour, reason: gap.reason.clone(), before: before.to_string() })
        })
        .collect();
    placed.sort_by_key(|(before, gap)| (*before, Reverse(gap.last_hour)));
    FoldedPage { total, offset: clamped, lines, gaps: placed.into_iter().map(|(_, gap)| gap).collect() }
}
