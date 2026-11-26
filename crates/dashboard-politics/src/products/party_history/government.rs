use super::{GovernmentCauseKind, GovernmentChange, complete_reads, number, unless_withheld};
use dashboard_query::{Query, cost};
use replay_data::rows::{GovernmentCause, PoliticsCaptureCoverageRow, TableId};
use replay_lanes::lanes::{self, Cell, History, Keys, Reading};
use std::io;

pub(super) struct Columns {
    link: usize,
    change: [usize; 4],
    witness: [usize; 3],
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
            change: fields(TableId::GovernmentChanges, ["hour", "before_ideology", "after_ideology", "cause"])?,
            witness: fields(TableId::PoliticsCaptureCoverage, ["since_hour", "through_hour", "streams"])?,
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

type Links = Vec<(u128, Vec<(i32, Option<u32>)>)>;

fn maps(steps: &[(i32, Option<u32>)], hour: i32, slot: u32) -> bool {
    steps.partition_point(|&(at, _)| at <= hour).checked_sub(1).is_some_and(|at| steps[at].1 == Some(slot))
}

pub(super) fn read(query: &Query<'_>, columns: &Columns, run: u32, slot: u32, (first, last): (i32, i32), changes: &mut Vec<GovernmentChange>, gaps: &mut Vec<(i32, i32)>) -> io::Result<()> {
    let recording = query.replay.recording();
    if [TableId::GovernmentChanges, TableId::PoliticsCaptureCoverage].iter().any(|table| recording.predates(table.index()).is_some()) {
        gaps.push((first, last));
        return Ok(());
    }
    let history = cost::storage(|| unless_withheld(lanes::history(recording, run, TableId::CountryLinks, &[columns.link], Keys::All, first, last)))?;
    let links: Links = match places(&history, &[columns.link]) {
        Some([at]) => history
            .keys
            .iter()
            .map(|(raw, steps)| (*raw, steps.iter().map(|step| (step.hour, step.cells.as_ref().and_then(|cells| number(cells[at])).and_then(|value| u32::try_from(value).ok()))).collect::<Vec<_>>()))
            .filter(|(_, steps)| steps.iter().any(|&(_, held)| held == Some(slot)))
            .collect(),
        None => Vec::new(),
    };
    if links.is_empty() {
        gaps.push((first, last));
        return Ok(());
    }
    let raws: Vec<u128> = links.iter().map(|(raw, _)| *raw).collect();
    let complete = cost::storage(|| unless_withheld(complete_reads(recording, run, TableId::GovernmentChanges, first, last)))?;
    let read_at = |hour: i32| complete.get(complete.partition_point(|&(_, to)| to < hour)).is_some_and(|&(from, _)| from <= hour);
    let witnesses = cost::storage(|| unless_withheld(lanes::history(recording, run, TableId::PoliticsCaptureCoverage, &columns.witness, Keys::These(&raws), first, last)))?;
    let mut covered = Vec::new();
    if let Some([since_at, through_at, streams_at]) = places(&witnesses, &columns.witness) {
        for (raw, steps) in &links {
            for (from, to, reading) in witnesses.readings(*raw) {
                let Reading::Cells(cells) = reading else { continue };
                let (Some(since), Some(through), Some(streams)) = (hour_of(cells[since_at]), hour_of(cells[through_at]), number(cells[streams_at])) else { continue };
                if (from..=to).contains(&through) && streams & i64::from(PoliticsCaptureCoverageRow::GOVERNMENT) != 0 && read_at(through) && maps(steps, through, slot) && since <= through {
                    covered.push((since.saturating_add(1).min(through).max(first), through.min(last)));
                }
            }
        }
    }
    covered.sort_unstable();
    let mut open = first;
    for (from, to) in covered {
        if from > open {
            gaps.push((open, from - 1));
        }
        open = open.max(to.saturating_add(1));
    }
    if open <= last {
        gaps.push((open, last));
    }
    let ranges: Vec<(u128, u128)> = raws.iter().map(|raw| (raw << 64, raw << 64 | u128::from(u64::MAX))).collect();
    let rows = cost::storage(|| unless_withheld(lanes::history(recording, run, TableId::GovernmentChanges, &columns.change, Keys::Ranges(&ranges), first, last)))?;
    let Some([hour_at, before_at, after_at, cause_at]) = places(&rows, &columns.change) else { return Ok(()) };
    let run_first = recording.run(run)?.first_hour;
    let mut found = Vec::new();
    for (key, steps) in &rows.keys {
        let Ok(at) = links.binary_search_by_key(&(key >> 64), |(raw, _)| *raw) else { continue };
        let Some((witnessed, cells)) = steps.iter().find_map(|step| Some((step.hour, step.cells.as_deref()?))) else { continue };
        if witnessed <= run_first || !maps(&links[at].1, witnessed, slot) {
            continue;
        }
        let Some(hour) = hour_of(cells[hour_at]) else { continue };
        let token = |cell: Cell| number(cell).and_then(|value| u32::try_from(value).ok());
        found.push((witnessed, hour, token(cells[before_at]), token(cells[after_at]), cause(number(cells[cause_at]))));
    }
    let mut words: Vec<u128> = found.iter().flat_map(|&(_, _, before, after, _)| [before, after]).flatten().map(u128::from).collect();
    words.sort_unstable();
    words.dedup();
    let lexicon = if words.is_empty() { History::default() } else { cost::storage(|| unless_withheld(lanes::history(recording, run, TableId::LexicalTokens, &[columns.literal], Keys::These(&words), first, last)))? };
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
