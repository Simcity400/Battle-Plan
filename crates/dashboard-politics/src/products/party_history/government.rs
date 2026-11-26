use super::{GovernmentCauseKind, GovernmentChange, number, unless_withheld};
use dashboard_query::{Query, cost};
use replay_data::rows::{GovernmentCause, PoliticsCaptureCoverageRow, TableId};
use replay_lanes::lanes::{self, Cell, History, Keys, Reading};
use std::{collections::BTreeSet, io};

pub(super) struct Columns {
    link: usize,
    change: [usize; 5],
    streams: usize,
    literal: usize,
}

fn fields<const N: usize>(table: TableId, names: [&str; N]) -> io::Result<[usize; N]> {
    let mut out = [0; N];
    for (at, name) in names.into_iter().enumerate() {
        out[at] = dashboard_declare::field(table, name)?;
    }
    Ok(out)
}

impl Columns {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self {
            link: dashboard_declare::field(TableId::CountryLinks, "canonical_slot")?,
            change: fields(TableId::GovernmentChanges, ["epoch", "hour", "before_ideology", "after_ideology", "cause"])?,
            streams: dashboard_declare::field(TableId::PoliticsCaptureCoverage, "streams")?,
            literal: dashboard_declare::field(TableId::LexicalTokens, "literal")?,
        })
    }
}

fn places<const N: usize>(history: &History, columns: &[usize; N]) -> Option<[usize; N]> {
    let mut out = [0; N];
    for (at, column) in columns.iter().enumerate() {
        out[at] = history.columns.binary_search(column).ok()?;
    }
    Some(out)
}

fn hour_of(cell: Cell) -> Option<i32> {
    number(cell).and_then(|value| i32::try_from(value).ok())
}

fn cause(raw: Option<i64>) -> Option<GovernmentCauseKind> {
    let cause = GovernmentCause::try_from(u8::try_from(raw?).ok()?).ok()?;
    match cause {
        GovernmentCause::NotIdentified => None,
        GovernmentCause::ElectionOnAction => Some(GovernmentCauseKind::Election),
        GovernmentCause::CoupCompletion => Some(GovernmentCauseKind::Coup),
        GovernmentCause::CivilWar => Some(GovernmentCauseKind::CivilWar),
        GovernmentCause::SetPoliticsEffect => Some(GovernmentCauseKind::Effect),
        GovernmentCause::PeaceForceGovernment => Some(GovernmentCauseKind::PeaceConference),
        GovernmentCause::SubjectGovernment => Some(GovernmentCauseKind::Overlord),
        GovernmentCause::Console => Some(GovernmentCauseKind::Console),
        GovernmentCause::ResetSetup => Some(GovernmentCauseKind::Reset),
    }
}

type Slots = Vec<(i32, Option<u32>)>;

fn maps(steps: &Slots, hour: i32, slot: u32) -> bool {
    steps.partition_point(|&(at, _)| at <= hour).checked_sub(1).is_some_and(|at| steps[at].1 == Some(slot))
}

fn push(out: &mut Vec<(i32, i32)>, first: i32, last: i32) {
    match out.last_mut() {
        Some(held) if i64::from(held.1) + 1 >= i64::from(first) => held.1 = held.1.max(last),
        _ => out.push((first, last)),
    }
}

fn mapped(steps: &Slots, slot: u32, (first, last): (i32, i32)) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for (at, &(hour, held)) in steps.iter().enumerate() {
        let end = steps.get(at + 1).map_or(last, |&(next, _)| next - 1);
        let (from, to) = (hour.max(first), end.min(last));
        if held == Some(slot) && from <= to {
            push(&mut out, from, to);
        }
    }
    out
}

fn intersect(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let (first, last) = (a[i].0.max(b[j].0), a[i].1.min(b[j].1));
        if first <= last {
            push(&mut out, first, last);
        }
        if a[i].1 < b[j].1 { i += 1 } else { j += 1 }
    }
    out
}

pub(super) fn read(query: &Query<'_>, columns: &Columns, run: u32, held: &[(i32, i32, u32)], (from, to): (i32, i32), changes: &mut Vec<GovernmentChange>, gaps: &mut Vec<(i32, i32)>) -> io::Result<()> {
    if held.is_empty() {
        return Ok(());
    }
    let recording = query.replay.recording();
    let unknown = |gaps: &mut Vec<(i32, i32)>| gaps.extend(held.iter().map(|&(first, last, _)| (first, last)));
    if [TableId::GovernmentChanges, TableId::PoliticsCaptureCoverage].iter().any(|table| recording.predates(table.index()).is_some()) {
        unknown(gaps);
        return Ok(());
    }
    let slots: BTreeSet<u32> = held.iter().map(|&(_, _, slot)| slot).collect();
    let history = cost::storage(|| unless_withheld(lanes::history(recording, run, TableId::CountryLinks, &[columns.link], Keys::All, from, to)))?;
    let links: Vec<(u128, Slots)> = match places(&history, &[columns.link]) {
        Some([at]) => history
            .keys
            .iter()
            .map(|(raw, steps)| (*raw, steps.iter().map(|step| (step.hour, step.cells.as_ref().and_then(|cells| number(cells[at])).and_then(|value| u32::try_from(value).ok()))).collect::<Slots>()))
            .filter(|(_, steps)| steps.iter().any(|&(_, slot)| slot.is_some_and(|slot| slots.contains(&slot))))
            .collect(),
        None => Vec::new(),
    };
    if links.is_empty() {
        unknown(gaps);
        return Ok(());
    }
    let raws: Vec<u128> = links.iter().map(|(raw, _)| *raw).collect();
    let drained = cost::storage(|| -> io::Result<_> {
        let changed = unless_withheld(lanes::complete_reads(recording, run, TableId::GovernmentChanges, from, to))?;
        let witnessed = unless_withheld(lanes::complete_reads(recording, run, TableId::PoliticsCaptureCoverage, from, to))?;
        Ok(intersect(&changed, &witnessed))
    })?;
    let witnesses = cost::storage(|| unless_withheld(lanes::history(recording, run, TableId::PoliticsCaptureCoverage, &[columns.streams], Keys::These(&raws), from, to)))?;
    let mut covered = vec![Vec::new(); held.len()];
    if let Some([at]) = places(&witnesses, &[columns.streams]) {
        for (raw, steps) in &links {
            let mut certified = Vec::new();
            for (first, last, reading) in witnesses.readings(*raw) {
                if let Reading::Cells(cells) = reading
                    && number(cells[at]).is_some_and(|streams| streams & i64::from(PoliticsCaptureCoverageRow::GOVERNMENT) != 0)
                {
                    push(&mut certified, first, last);
                }
            }
            let stamped: Vec<(i32, i32)> = intersect(&certified, &drained).into_iter().filter(|&(first, last)| first < last).map(|(first, last)| (first, last - 1)).collect();
            for (spans, &(first, last, slot)) in covered.iter_mut().zip(held) {
                spans.extend(intersect(&stamped, &mapped(steps, slot, (first, last))));
            }
        }
    }
    for (mut spans, &(first, last, _)) in covered.into_iter().zip(held) {
        spans.sort_unstable();
        let mut open = first;
        for (start, end) in spans {
            if start > open {
                gaps.push((open, start - 1));
            }
            open = open.max(end.saturating_add(1));
        }
        if open <= last {
            gaps.push((open, last));
        }
    }
    let ranges: Vec<(u128, u128)> = raws.iter().map(|raw| (raw << 64, raw << 64 | u128::from(u64::MAX))).collect();
    let rows = cost::storage(|| unless_withheld(lanes::history(recording, run, TableId::GovernmentChanges, &columns.change, Keys::Ranges(&ranges), from, to)))?;
    let Some([epoch_at, hour_at, before_at, after_at, cause_at]) = places(&rows, &columns.change) else { return Ok(()) };
    let token = |cell: Cell| number(cell).and_then(|value| u32::try_from(value).ok());
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for (key, steps) in &rows.keys {
        let Ok(at) = raws.binary_search(&(key >> 64)) else { continue };
        for step in steps {
            let Some(cells) = step.cells.as_deref() else { continue };
            let (Some(epoch), Some(hour)) = (number(cells[epoch_at]), hour_of(cells[hour_at])) else { continue };
            if !seen.insert((*key, epoch)) || !held.iter().any(|&(first, last, slot)| (first..=last).contains(&hour) && maps(&links[at].1, hour, slot)) {
                continue;
            }
            found.push((step.hour, hour, token(cells[before_at]), token(cells[after_at]), cause(number(cells[cause_at]))));
        }
    }
    let mut words: Vec<u128> = found.iter().flat_map(|&(_, _, before, after, _)| [before, after]).flatten().map(u128::from).collect();
    words.sort_unstable();
    words.dedup();
    let lexicon = if words.is_empty() { History::default() } else { cost::storage(|| unless_withheld(lanes::history(recording, run, TableId::LexicalTokens, &[columns.literal], Keys::These(&words), from, to)))? };
    let literal = places(&lexicon, &[columns.literal]);
    let word = |token: Option<u32>, witnessed: i32| -> Option<String> {
        let [at] = literal?;
        match lexicon.reading(u128::from(token?), witnessed) {
            Reading::Cells(cells) => lexicon.text(cells[at]).and_then(|text| std::str::from_utf8(text).ok()).filter(|text| !text.is_empty()).map(str::to_owned),
            _ => None,
        }
    };
    for (witnessed, hour, before, after, cause) in found {
        changes.push(GovernmentChange { hour, witnessed, before: word(before, witnessed), after: word(after, witnessed), cause });
    }
    Ok(())
}
