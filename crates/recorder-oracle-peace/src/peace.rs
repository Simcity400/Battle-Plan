#[cfg(test)]
#[path = "peace_tests.rs"]
mod tests;
use super::*;
use installed_content_save::projects;
use replay_data::{rows::PeaceSettlementRow, rows::SettledPeaceAction};

pub const DESTROYED_ACTIONS: &str = "PeaceSettlements destroyed-actions limitation: normal finalization destroys bidding turns and ordered actions before a completed save; a completed save cannot independently certify action types, order, negotiators or operands.";

fn equal<T: std::fmt::Debug + PartialEq>(
    report: &mut Report,
    label: String,
    captured: T,
    saved: T,
) {
    if captured != saved {
        report.difference(label, format!("{captured:?}"), format!("{saved:?}"));
    }
}

fn conference_label(conference: Option<u64>) -> String {
    conference.map_or_else(|| "with no recorded identity".into(), |id| id.to_string())
}

fn available(row: &PeaceSettlementRow) -> io::Result<()> {
    if row.failure.is_some() || row.omitted != 0 {
        return Err(invalid(format!(
            "conference {} unavailable (at least {} unread operands): {}",
            conference_label(row.conference),
            row.omitted,
            String::from_utf8_lossy(row.failure.as_deref().unwrap_or_default())
        )));
    }
    Ok(())
}

pub fn compare_bids(
    frame: &DecodedFrame,
    saved: &projects::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::PeaceBids>(frame)?;
    let conferences = saved
        .conferences
        .as_ref()
        .ok_or_else(|| invalid("save lacks peace conference manager"))?;
    let countries = country_context(frame)?;
    let tokens = lexical_tokens(frame, report)?;
    let mut seen = BTreeSet::new();
    let mut count = 0;
    for row in rows {
        available(row)?;
        if row.hour != Some(frame.hour) {
            return Err(invalid("peace bids are not from the oracle hour"));
        }
        let id = row
            .conference
            .filter(|&id| id != 0)
            .ok_or_else(|| invalid("peace bids lack conference identity"))?;
        if !seen.insert(id) {
            return Err(invalid("duplicate peace bid conference"));
        }
        let Some(conference) = conferences.get(&id) else {
            report.difference(format!("PeaceBids {id}"), "recorded", "absent from save");
            continue;
        };
        let mut groups: BTreeMap<&[u8], Vec<&SettledPeaceAction>> = BTreeMap::new();
        let mut group_order = Vec::new();
        for action in &row.actions {
            let tag = country_tag(action.country, &countries.links)?
                .ok_or_else(|| invalid("peace bid country is null"))?;
            groups.entry(tag).or_default().push(action);
            if group_order.last().copied() != Some(tag) {
                group_order.push(tag);
            }
        }
        let saved_order = conference
            .bid_countries
            .iter()
            .filter(|tag| {
                conference
                    .bids
                    .get(*tag)
                    .is_some_and(|turns| turns.iter().any(|turn| !turn.actions.is_empty()))
            })
            .map(Vec::as_slice)
            .collect::<Vec<_>>();
        equal(
            report,
            format!("PeaceBids {id}.country_group_order"),
            group_order,
            saved_order,
        );
        for (tag, turns) in &conference.bids {
            let actual = groups.remove(tag.as_slice()).unwrap_or_default();
            let expected_count = turns.iter().map(|turn| turn.actions.len()).sum::<usize>();
            equal(
                report,
                format!(
                    "PeaceBids {id} {}.action_count",
                    String::from_utf8_lossy(tag)
                ),
                actual.len(),
                expected_count,
            );
            let mut index = 0;
            for (turn, bid) in turns.iter().enumerate() {
                for (ordinal, saved) in bid.actions.iter().enumerate() {
                    let Some(action) = actual.get(index) else {
                        index += 1;
                        continue;
                    };
                    index += 1;
                    let label = format!(
                        "PeaceBids {id} {} turn {turn} action {ordinal}",
                        String::from_utf8_lossy(tag)
                    );
                    equal(report, format!("{label}.turn"), action.turn as usize, turn);
                    equal(
                        report,
                        format!("{label}.ordinal"),
                        action.ordinal as usize,
                        ordinal,
                    );
                    equal(
                        report,
                        format!("{label}.country"),
                        country_tag(action.country, &countries.links)?,
                        bid.country.as_deref(),
                    );
                    let token = tokens.get(&action.token).copied().ok_or_else(|| {
                        invalid("peace action token is not in the recorded dictionary")
                    })?;
                    equal(
                        report,
                        format!("{label}.token"),
                        token,
                        saved.token.as_slice(),
                    );
                    if let Some(name) = &action.cached_name {
                        equal(
                            report,
                            format!("{label}.cached_name"),
                            Some(name.as_slice()),
                            saved.name.as_deref(),
                        );
                    } else {
                        report.limitation(
                            "Peace action cached type name is not recorded",
                            label.clone(),
                        );
                    }
                    for (name, raw, expected) in [
                        ("taker", action.taker, &saved.taker),
                        ("giver", action.giver, &saved.giver),
                        ("negotiator", action.negotiator, &saved.negotiator),
                    ] {
                        equal(
                            report,
                            format!("{label}.{name}"),
                            country_tag(raw, &countries.links)?,
                            expected.as_deref(),
                        );
                    }
                    equal(
                        report,
                        format!("{label}.status"),
                        action.status,
                        saved.status,
                    );
                    equal(report, format!("{label}.cost"), action.cost, saved.cost);
                    equal(report, format!("{label}.state"), action.state, saved.state);
                    equal(
                        report,
                        format!("{label}.flags"),
                        action.flags.map(|flags| flags.map(|flag| flag != 0)),
                        (saved.state.is_some() || saved.flags.iter().any(|&flag| flag))
                            .then_some(saved.flags),
                    );
                    let operand = action
                        .country_operand
                        .map(|raw| country_tag(raw, &countries.links))
                        .transpose()?
                        .flatten();
                    equal(
                        report,
                        format!("{label}.country_operand"),
                        operand,
                        saved.puppet.as_deref().or(saved.liberate.as_deref()),
                    );
                    equal(
                        report,
                        format!("{label}.ship"),
                        action.ship.filter(|&id| id != 0),
                        saved.ship,
                    );
                    match (action.ratio, &saved.ratio) {
                        (Some(raw), Some(decimal)) => {
                            equal(report, format!("{label}.ratio"), raw, decimal.raw);
                        }
                        (None, None) => {}
                        _ => report.difference(
                            format!("{label}.ratio"),
                            format!("{:?}", action.ratio),
                            format!("{:?}", saved.ratio),
                        ),
                    }
                    count += 1;
                }
            }
        }
        for tag in groups.keys() {
            report.difference(
                format!("PeaceBids {id} {}", String::from_utf8_lossy(tag)),
                "recorded actions",
                "absent from save history",
            );
        }
    }
    for id in conferences.keys() {
        if !seen.contains(id) {
            report.difference(
                format!("PeaceBids {id}"),
                "absent from replay",
                "saved conference",
            );
        }
    }
    report.compared("PeaceBids", count);
    report.limitation("peace bids compare nonempty country groups in writer order and each turn's ordered actions and cached type names; tag text cannot certify raw reference bits; enabled flags cannot certify noncanonical raw bytes; empty groups and turns, participant aggregates, finalized hour and puppet auxiliary are not action operands compared here", "PeaceBids");
    Ok(())
}

pub fn compare_recorded_settlements(
    replay: &Replay,
    frame: &DecodedFrame,
    save: &Save,
    report: &mut Report,
) -> io::Result<()> {
    let first_hour = replay
        .branches
        .iter()
        .find(|b| b.id == frame.branch)
        .map(|b| b.first_hour);
    let event_losses: Vec<_> = replay.event_losses(frame.branch).collect();
    let crossed = std::cell::Cell::new(None);
    let preceding = |hour| -> io::Result<Option<Box<DecodedFrame>>> {
        if first_hour.is_none_or(|first| hour < first) {
            return Ok(None);
        }
        if let Some(&loss) = event_losses.iter().find(|&&(after, _)| after == hour) {
            crossed.set(Some(loss));
            return Ok(None);
        }
        match replay.seek(frame.branch, hour)? {
            SeekResult::Frame(previous)
                if previous.branch == frame.branch && previous.hour == hour =>
            {
                Ok(Some(previous))
            }
            _ => Err(invalid(
                "previous hourly PeaceBids sample unavailable; finalized-conference completeness cannot be certified",
            )),
        }
    };
    compare_settlements_after(frame, preceding, save, report)?;
    if let Some((after, lost)) = crossed.get() {
        report.limitation(
            "a recorded event loss (the recorder was detached) lies between these reads; conferences resident before it and finalized across it have no certified finalizer capture",
            format!("PeaceSettlements between the reads of hours {after} and {lost}"),
        );
    }
    Ok(())
}

fn drain_busy(frame: &DecodedFrame) -> bool {
    matches!(&*frame.tables[TableId::PeaceSettlements.index()], TableView::Unavailable { reason }
        if reason == replay_data::rows::PEACE_DRAIN_BUSY)
}

#[cfg(test)]
fn compare_settlements(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    compare_settlements_after(frame, |_| Ok(None::<&DecodedFrame>), save, report)
}

fn preceding_finalizations<F: std::borrow::Borrow<DecodedFrame>>(
    branch: BranchId,
    mut hour: Hour,
    mut resident: BTreeSet<u64>,
    preceding: &mut impl FnMut(Hour) -> io::Result<Option<F>>,
) -> io::Result<(BTreeSet<u64>, BTreeSet<u64>)> {
    let mut finalized = BTreeSet::new();
    let mut before_observation = BTreeSet::new();
    while !resident.is_empty() {
        let Some(prior) = preceding(hour)? else {
            break;
        };
        let prior = prior.borrow();
        if prior.branch != branch || prior.hour != hour {
            return Err(invalid(
                "previous peace sample is not the requested branch/hour",
            ));
        }
        let ids = complete::<tables::PeaceBids>(prior)?
            .iter()
            .map(|bid| {
                bid.conference
                    .filter(|&id| id != 0)
                    .ok_or_else(|| invalid("previous peace bid lacks conference identity"))
            })
            .collect::<io::Result<BTreeSet<_>>>()?;
        resident.retain(|id| ids.contains(id));
        if !resident.is_empty() && !drain_busy(prior) {
            for row in complete::<tables::PeaceSettlements>(prior)? {
                let Some(id) = row.conference.filter(|id| resident.contains(id)) else {
                    continue;
                };
                available(row)?;
                let observed = row
                    .hour
                    .ok_or_else(|| invalid("peace settlement hour unavailable"))?;
                let mut earliest = hour.saturating_sub(1);
                if observed < earliest {
                    let mut check_hour = hour.checked_sub(1);
                    while let Some(at) = check_hour {
                        let Some(witness) = preceding(at)? else {
                            break;
                        };
                        let witness = witness.borrow();
                        if witness.branch != branch || witness.hour != at {
                            return Err(invalid(
                                "previous peace sample is not the requested branch/hour",
                            ));
                        }
                        complete::<tables::PeaceBids>(witness)?;
                        earliest = at;
                        if !drain_busy(witness) {
                            break;
                        }
                        check_hour = at.checked_sub(1);
                    }
                }
                if observed > hour || observed < earliest {
                    return Err(invalid(
                        "settlement publication is outside the recorded busy-drain span",
                    ));
                }
                resident.remove(&id);
                finalized.insert(id);
            }
        }
        before_observation = prior
            .complete::<tables::PeaceConferences>()
            .map(|rows| {
                rows.iter()
                    .filter(|row| row.finalized != 0 && resident.contains(&row.id))
                    .map(|row| row.id)
                    .collect()
            })
            .unwrap_or_default();
        let Some(previous) = hour.checked_sub(1) else {
            break;
        };
        hour = previous;
    }
    Ok((finalized, before_observation))
}

fn compare_settlements_after<F: std::borrow::Borrow<DecodedFrame>>(
    frame: &DecodedFrame,
    mut preceding: impl FnMut(Hour) -> io::Result<Option<F>>,
    save: &Save,
    report: &mut Report,
) -> io::Result<()> {
    report.limitation(DESTROYED_ACTIONS, "PeaceSettlements");
    let (census_rows, losses) = match &*frame.tables[TableId::PeaceSettlements.index()] {
        TableView::Complete(rows) => (rows.rows::<tables::PeaceSettlements>().unwrap(), 0),
        TableView::Truncated { rows, missing } => {
            let rows = rows.rows::<tables::PeaceSettlements>().unwrap();
            let operands = rows.iter().fold(0_u64, |n, r| n.saturating_add(r.omitted));
            (rows, u64::from(*missing).saturating_sub(operands))
        }
        TableView::Unavailable { .. } if drain_busy(frame) => (&[][..], 0),
        TableView::Unavailable { reason } => return Err(invalid(reason.clone())),
        TableView::NotRecorded { mode } => {
            return Err(invalid(format!(
                "PeaceSettlements is not recorded in a {mode} recording"
            )));
        }
    };
    let captured = census_rows
        .iter()
        .filter_map(|r| r.conference)
        .collect::<BTreeSet<_>>();
    let mut disappeared = BTreeSet::new();
    let mut earliest_hour = frame.hour.saturating_sub(1);
    let mut preceding_hour = frame.hour.checked_sub(1);
    while let Some(hour) = preceding_hour {
        let Some(prior) = preceding(hour)? else {
            report.limitation("previous hourly PeaceBids sample unavailable; finalized-conference completeness cannot be certified", "PeaceSettlements");
            break;
        };
        let prior = prior.borrow();
        if prior.branch != frame.branch || prior.hour != hour {
            return Err(invalid(
                "previous peace sample is not the requested branch/hour",
            ));
        }
        earliest_hour = hour;
        let roster = save.projects.conferences.as_ref().ok_or_else(|| {
            invalid("save lacks peace conference manager for settlement completeness")
        })?;
        for bid in complete::<tables::PeaceBids>(prior)? {
            let id = bid
                .conference
                .filter(|&id| id != 0)
                .ok_or_else(|| invalid("previous peace bid lacks conference identity"))?;
            if !roster.contains_key(&id) && !captured.contains(&id) {
                disappeared.insert(id);
            }
        }
        if !drain_busy(prior) {
            break;
        }
        preceding_hour = hour.checked_sub(1);
    }
    let (finalized, before_observation) = preceding_finalizations(
        frame.branch,
        earliest_hour,
        disappeared.clone(),
        &mut preceding,
    )?;
    disappeared.retain(|id| !finalized.contains(id) && !before_observation.contains(id));
    for id in before_observation {
        report.limitation(
            "conference already finalized at the branch's first resident read; its pre-observation finalizer capture cannot be certified",
            format!("PeaceSettlements {id}"),
        );
    }
    report.compared(
        "PeaceSettlements previously finalized resident conferences",
        finalized.len(),
    );
    if drain_busy(frame) {
        report.deferred_tables.insert(
            TableId::PeaceSettlements,
            replay_data::rows::PEACE_DRAIN_BUSY.into(),
        );
        report.compared(
            "PeaceSettlements conferences deferred by busy drain",
            disappeared.len(),
        );
        report.limitation(
            "peace finalizer busy lease at drain defers owned publication until a successful drain",
            "PeaceSettlements",
        );
        return Ok(());
    }
    let mut counted = 0_u64;
    for id in &disappeared {
        if counted < losses {
            counted += 1;
        } else {
            report.difference(
                format!("PeaceSettlements {id}.finalized capture"),
                "no settlement row or counted loss",
                "previous PeaceBids conference absent from save roster",
            );
        }
    }
    report.compared(
        "PeaceSettlements finalized conferences with counted losses",
        counted as usize,
    );
    report.compared(
        "PeaceSettlements finalized conference census",
        disappeared.len() + captured.len() + finalized.len(),
    );
    let countries = country_context(frame)?;
    let rows = complete::<tables::PeaceSettlements>(frame)?;
    let mut states: BTreeMap<i32, u32> = BTreeMap::new();
    let mut subjects = BTreeMap::new();
    let mut previous_sequence = None;
    for row in rows {
        available(row)?;
        if row.conference.is_none_or(|id| id == 0) {
            return Err(invalid("peace settlement identity unavailable"));
        }
        if previous_sequence.is_some_and(|sequence| row.sequence <= sequence) {
            return Err(invalid(
                "peace settlement sequence is not strictly increasing",
            ));
        }
        previous_sequence = Some(row.sequence);
        let hour = row
            .hour
            .ok_or_else(|| invalid("peace settlement hour unavailable"))?;
        if hour > frame.hour || hour < earliest_hour {
            return Err(invalid(
                "settlement publication is outside the recorded busy-drain span",
            ));
        }
        if hour < frame.hour.saturating_sub(1) {
            report.compared("PeaceSettlements events published after busy drain", 1);
            report.limitation(
                "peace finalizer busy leases at preceding drains deferred this event until the recorded publication hour",
                format!(
                    "PeaceSettlements {}: finalized hour {hour}, published hour {}",
                    conference_label(row.conference),
                    frame.hour
                ),
            );
        }
        for transfer in &row.transfers {
            states.insert(transfer.state, transfer.taker);
        }
        for subject in &row.subjects {
            subjects.insert(subject.subject, subject.overlord);
        }
    }
    for (state, raw) in &states {
        let id =
            u32::try_from(*state).map_err(|_| invalid("peace outcome has a negative state id"))?;
        let tag = country_tag(*raw, &countries.links)?
            .ok_or_else(|| invalid("settled state recipient is null"))?;
        let saved = save
            .states
            .get(&id)
            .ok_or_else(|| invalid(format!("settled state {id} absent from save")))?;
        equal(
            report,
            format!("PeaceSettlements state {id}.owner"),
            Some(tag),
            saved.owner.as_deref(),
        );
    }
    let mut saved_subjects: BTreeMap<&[u8], BTreeSet<&[u8]>> = BTreeMap::new();
    for country in save.intel.countries.values() {
        if let Some(subjects) = country.diplomacy.as_ref().and_then(|d| d.subjects.as_ref()) {
            for subject in subjects {
                if !subject.cancel && subject.end_date.is_none() {
                    saved_subjects
                        .entry(subject.second.as_slice())
                        .or_default()
                        .insert(subject.first.as_slice());
                }
            }
        }
    }
    for (subject, overlord) in &subjects {
        let first = country_tag(*overlord, &countries.links)?
            .ok_or_else(|| invalid("settled overlord is null"))?;
        let second = country_tag(*subject, &countries.links)?
            .ok_or_else(|| invalid("settled subject is null"))?;
        equal(
            report,
            format!(
                "PeaceSettlements {}.subject overlords",
                String::from_utf8_lossy(second)
            ),
            BTreeSet::from([first]),
            saved_subjects.get(second).cloned().unwrap_or_default(),
        );
    }
    report.compared("PeaceSettlements retained state owners", states.len());
    report.compared("PeaceSettlements retained subjects", subjects.len());
    report.limitation("settlement outcomes compare final recipients with the first following save; unrelated scripted outcomes and earlier intermediate transfers have no independent settlement attribution in a completed save", "PeaceSettlements");
    Ok(())
}
