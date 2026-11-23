use super::Replay;
use replay_recording::records::{Records, RecoveryNote};
use replay_data::{BranchId, codec::invalid_static as invalid, identity::{Identity, SessionStart}};
use replay_frame::branches::BranchStart;
use replay_recording::control::ControlRecord;
use replay_capture_spool::spool::{RecordRef, Recovery, SegmentState, SpoolRecord};
use std::{collections::BTreeMap, io};

pub fn records_of<R: SpoolRecord>(
    segments: &[(u32, Recovery<R>)],
    identity: &Identity,
    campaign_id: &[u8],
    existing: Option<&Replay>,
) -> io::Result<Records> {
    identity.validate()?;
    let mut sorted: Vec<_> = segments.iter().collect();
    sorted.sort_unstable_by_key(|(number, _)| *number);
    if sorted.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid("duplicate spool segment number"));
    }
    let mut sessions: BTreeMap<u64, SessionStart> = existing
        .into_iter()
        .flat_map(|replay| &replay.sessions)
        .map(|session| (session.id, session.clone()))
        .collect();
    let mut branches: BTreeMap<BranchId, BranchStart> = existing
        .into_iter()
        .flat_map(|replay| &replay.branches)
        .map(|branch| (branch.id, branch.clone()))
        .collect();
    let mut held_controls = std::collections::HashSet::new();
    for control in existing.into_iter().flat_map(|replay| &replay.controls) {
        let mut encoded = Vec::new();
        control.encode(&mut encoded)?;
        held_controls.insert(encoded);
    }
    let mut records = Records::default();
    for (number, recovery) in &sorted {
        if let Some(reason) = &recovery.damage {
            let session = recovery.records.first().and_then(|record| match record.view() {
                RecordRef::SessionStart(session) => Some(session.id),
                RecordRef::BranchStart(branch) => Some(branch.session),
                _ => None,
            });
            engine_allocation::reserve(&mut records.notes, 1)?;
            records.notes.push(RecoveryNote {
                session,
                segment: *number,
                valid_bytes: recovery.valid_bytes as u64,
                reason: reason.clone(),
            });
        }
        let segment = &recovery.records;
        let Some(first) = segment.first() else {
            continue;
        };
        if !matches!(first.view(), RecordRef::SessionStart(_) | RecordRef::BranchStart(_)) {
            return Err(invalid("segment does not begin with session or branch context"));
        }
        let mut current_session = None;
        let mut current_branch = None;
        let mut state = SegmentState::default();
        for record in segment {
            let record = record.view();
            state.accept_view(&record)?;
            match record {
                RecordRef::SessionStart(session) => {
                    if !session.identity.same_content(identity) {
                        return Err(invalid("session content differs from replay content"));
                    }
                    if sessions
                        .values()
                        .any(|old| old.capture_mode != session.capture_mode)
                    {
                        return Err(invalid(
                            "session capture mode differs from the replay's; record into a new file",
                        ));
                    }
                    match sessions.get(&session.id) {
                        Some(previous) if previous != session => {
                            return Err(invalid("conflicting session context"));
                        }
                        Some(_) => {}
                        None => {
                            session.validate()?;
                            sessions.insert(session.id, session.clone());
                            engine_allocation::reserve(&mut records.sessions, 1)?;
                            records.sessions.push(session.clone());
                        }
                    }
                    current_session = Some(session.id);
                    current_branch = None;
                }
                RecordRef::BranchStart(branch) => {
                    branch.validate()?;
                    if !sessions.contains_key(&branch.session) {
                        return Err(invalid("branch refers to an unknown session"));
                    }
                    if current_session.is_some_and(|session| session != branch.session) {
                        return Err(invalid("branch changed session without session context"));
                    }
                    match branches.get(&branch.id) {
                        Some(previous) if previous != branch => {
                            return Err(invalid("conflicting branch context"));
                        }
                        Some(_) => {}
                        None => {
                            branches.insert(branch.id, branch.clone());
                            engine_allocation::reserve(&mut records.branches, 1)?;
                            records.branches.push(branch.clone());
                        }
                    }
                    current_session = Some(branch.session);
                    current_branch = Some(branch.id);
                }
                RecordRef::Frame(_) | RecordRef::Metadata(_) => {
                    let (branch, hour, _) = record.frame().expect("a frame record");
                    if current_branch != Some(branch) {
                        return Err(invalid("frame has no matching branch context"));
                    }
                    if hour < branches[&branch].first_hour {
                        return Err(invalid("frame precedes its branch start"));
                    }
                }
                RecordRef::Gap(gap) => {
                    gap.validate()?;
                    if current_branch != Some(gap.branch) {
                        return Err(invalid("gap has no matching branch context"));
                    }
                    if !records.gaps.contains(gap)
                        && !existing.is_some_and(|replay| replay.gaps.contains(gap))
                    {
                        engine_allocation::reserve(&mut records.gaps, 1)?;
                        records.gaps.push(gap.try_clone()?);
                    }
                }
                RecordRef::Control(control) => {
                    if current_session != Some(control.session()) {
                        return Err(invalid("control has no matching session context"));
                    }
                    declare_command_branch(control, campaign_id, &mut branches, &mut records)?;
                    let mut encoded = Vec::new();
                    control.encode(&mut encoded)?;
                    let repeated_loss = matches!(control, ControlRecord::EventLoss { .. })
                        && records.controls.contains(control);
                    if !repeated_loss && !held_controls.contains(&encoded) {
                        engine_allocation::reserve(&mut records.controls, 1)?;
                        records.controls.push(control.clone());
                    }
                }
            }
        }
    }
    Ok(records)
}

fn declare_command_branch(
    control: &ControlRecord,
    campaign_id: &[u8],
    branches: &mut BTreeMap<BranchId, BranchStart>,
    records: &mut Records,
) -> io::Result<()> {
    if let ControlRecord::PlayerCommands { session, packet } = control
        && let Some(phase) = &packet.phase
    {
        phase.validate()?;
        if phase.start.session != *session
            || phase.start.id != packet.branch
            || !phase.campaign.eq_ignore_ascii_case(campaign_id)
        {
            return Err(invalid(
                "command phase has conflicting campaign or branch context",
            ));
        }
        match branches.get(&phase.start.id) {
            Some(previous) if previous != &phase.start => {
                return Err(invalid("conflicting command branch declaration"));
            }
            Some(_) => {}
            None => {
                branches.insert(phase.start.id, phase.start.clone());
                engine_allocation::reserve(&mut records.branches, 1)?;
                records.branches.push(phase.start.clone());
            }
        }
    }
    Ok(())
}
