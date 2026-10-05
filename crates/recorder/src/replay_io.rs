//! Passive finalization and inspection use the same codecs as the collector.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use replay::{
    BranchId, Hour, SessionId,
    branches::{Gap, Parentage},
    container::{RecoveryNote, Replay, SeekResult},
    frame::{DecodedFrame, Frame, FrameDecoder, TABLE_COUNT, TableUpdate, TableView},
    identity::{Identity, SessionStart, Verification, is_campaign_uuid},
    rows::{TableId, TableRows, tables},
    spool::{ControlRecord, Record, Recovery, recover_prefix},
    witness::{TerminalWitness, WITNESS_BYTES, WITNESS_FILE_NAME},
};

type Result<T> = std::result::Result<T, String>;

const ALLOCATION_GAP_REASON: &str = "collector allocation failed";

/// A prepared identity record means no collector failure was published.
pub(crate) fn read_failure_witness(session_dir: &Path) -> Result<Option<TerminalWitness>> {
    let path = session_dir.join(WITNESS_FILE_NAME);
    let config_path = session_dir.join("bin/collector.json");
    let modern = config_path.is_file();
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if modern {
                return Err(failure(&path, "precreated terminal witness is missing"));
            }
            return Ok(None);
        }
        Err(error) => return Err(failure(&path, error)),
    };
    if bytes.len() != WITNESS_BYTES {
        return Err(failure(&path, "terminal witness has the wrong length"));
    }
    if bytes.iter().all(|byte| *byte == 0) {
        if modern {
            return Err(failure(&path, "modern terminal witness is uninitialised"));
        }
        return Ok(None);
    }
    if let Ok(identity) = TerminalWitness::decode_prepared(&bytes) {
        if modern {
            let config: serde_json::Value = serde_json::from_slice(
                &fs::read(&config_path).map_err(|error| failure(&config_path, error))?,
            )
            .map_err(|error| failure(&config_path, error))?;
            let session_id = config["session_id"].as_u64().ok_or_else(|| {
                failure(&config_path, "collector configuration has no session id")
            })?;
            let session_start: Vec<u8> = serde_json::from_value(config["session_start"].clone())
                .map_err(|error| failure(&config_path, error))?;
            let mut decoder = replay::codec::Decoder::new(&session_start);
            let session =
                SessionStart::decode(&mut decoder).map_err(|error| failure(&config_path, error))?;
            decoder
                .finish()
                .map_err(|error| failure(&config_path, error))?;
            if identity != (session_id, session.process_start_time) || session_id != session.id {
                return Err(failure(
                    &path,
                    "prepared witness belongs to a different process session",
                ));
            }
        }
        return Ok(None);
    }
    TerminalWitness::decode(&bytes)
        .map(Some)
        .map_err(|error| failure(&path, error))
}

fn failure(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

fn segment_number(name: &str) -> Result<Option<u32>> {
    if !name.starts_with("spool.") || !name.ends_with(".h4s") {
        return Ok(None);
    }
    let number = &name[6..name.len() - 4];
    if number.len() < 4 || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("invalid spool segment name: {name}"));
    }
    number
        .parse()
        .map(Some)
        .map_err(|_| format!("spool segment number exceeds u32: {name}"))
}

fn segment_paths(directory: &Path) -> Result<Vec<(u32, PathBuf)>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| failure(directory, error))? {
        let entry = entry.map_err(|error| failure(directory, error))?;
        if !entry
            .file_type()
            .map_err(|error| failure(&entry.path(), error))?
            .is_file()
        {
            continue;
        }
        if let Some(number) = segment_number(&entry.file_name().to_string_lossy())? {
            paths.push((number, entry.path()));
        }
    }
    paths.sort_unstable_by_key(|(number, _)| *number);
    if paths.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(failure(directory, "duplicate spool segment number"));
    }
    Ok(paths)
}

fn campaign_id(segments: &[(u32, Recovery)]) -> Result<Vec<u8>> {
    segments
        .iter()
        .flat_map(|(_, recovery)| &recovery.records)
        .find_map(|record| match record {
            Record::Frame(frame) => match &frame.tables[TableId::Campaign.index()] {
                TableUpdate::Snapshot(TableRows::Campaign(rows)) => match rows.as_slice() {
                    [campaign] if !campaign.id.is_empty() => Some(campaign.id.clone()),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        })
        .ok_or_else(|| "no complete campaign identity survived in the spool; directory names cannot establish identity".into())
}

fn campaign_filename(id: &[u8]) -> Result<&str> {
    if !is_campaign_uuid(id) {
        return Err(
            "recorded campaign identity is not a UUID; refusing to use it as a file name".into(),
        );
    }
    std::str::from_utf8(id).map_err(|error| error.to_string())
}

/// Call only after detach and module-absence confirmation, or after process exit.
pub fn finalize_session(session_dir: &Path, replay_dir: &Path) -> Result<Vec<PathBuf>> {
    let (paths, errors) = finalize_session_outcome(session_dir, replay_dir)?;
    if errors.is_empty() {
        Ok(paths)
    } else {
        Err(errors.join("\n"))
    }
}

/// Keep successfully written campaigns visible when another campaign is incomplete.
pub(crate) fn finalize_session_outcome(
    session_dir: &Path,
    replay_dir: &Path,
) -> Result<(Vec<PathBuf>, Vec<String>)> {
    let witness = read_failure_witness(session_dir)?;
    if witness.is_none()
        && ["status.json", "completed.json"].iter().any(|name| {
            fs::read(session_dir.join(name))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .is_some_and(|status| status["capture_failed"] == true)
        })
    {
        return Err(failure(
            session_dir,
            "capture failed without a valid terminal witness; spool retained",
        ));
    }
    fs::create_dir_all(replay_dir).map_err(|error| failure(replay_dir, error))?;
    let mut campaigns = Vec::new();
    for entry in fs::read_dir(session_dir).map_err(|error| failure(session_dir, error))? {
        let entry = entry.map_err(|error| failure(session_dir, error))?;
        if entry
            .file_type()
            .map_err(|error| failure(&entry.path(), error))?
            .is_dir()
        {
            let segments = segment_paths(&entry.path())?;
            if !segments.is_empty() {
                campaigns.push((entry.path(), segments));
            }
        }
    }
    campaigns.sort_by(|left, right| left.0.cmp(&right.0));
    if campaigns.is_empty() {
        return Err(failure(session_dir, "no campaign spool segments found"));
    }
    let mut outputs = Vec::new();
    let mut errors = Vec::new();
    let mut process_context = None;
    for (campaign_dir, paths) in campaigns {
        match finalize_campaign(
            &campaign_dir,
            &paths,
            replay_dir,
            &mut process_context,
            witness.as_ref(),
        ) {
            Ok(path) => outputs.push(path),
            Err(error) => errors.push(error),
        }
    }
    Ok((outputs, errors))
}

fn finalize_campaign(
    campaign_dir: &Path,
    paths: &[(u32, PathBuf)],
    replay_dir: &Path,
    process_context: &mut Option<(SessionId, u32, u64, u64)>,
    witness: Option<&TerminalWitness>,
) -> Result<PathBuf> {
    let started = Instant::now();
    let mut spool_bytes = 0_u64;
    let mut segments = Vec::with_capacity(paths.len());
    let failing_campaign = witness.is_some_and(|witness| {
        witness.campaign_id.as_ref().is_some_and(|campaign| {
            campaign_dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::as_bytes)
                == Some(campaign.as_slice())
        })
    });
    let committed_campaign = witness.is_some_and(|witness| {
        witness
            .committed_campaign_id
            .as_ref()
            .is_some_and(|campaign| {
                campaign_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::as_bytes)
                    == Some(campaign.as_slice())
            })
    });
    let pending_campaign = witness.is_some_and(|witness| {
        witness
            .pending_campaign_id
            .as_ref()
            .is_some_and(|campaign| {
                campaign_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::as_bytes)
                    == Some(campaign.as_slice())
            })
    });
    if witness.is_some_and(|witness| witness.committed_campaign_id.is_none())
        || (pending_campaign && witness.unwrap().pending_segment.is_none())
    {
        return Err(failure(
            campaign_dir,
            "failure witness has no committed spool watermark",
        ));
    }
    let watermark = witness.and_then(|witness| {
        let committed = committed_campaign
            .then(|| (witness.committed_segment.unwrap(), witness.committed_bytes));
        let pending =
            pending_campaign.then(|| (witness.pending_segment.unwrap(), witness.pending_bytes));
        match (committed, pending) {
            (Some(committed), Some(pending)) => Some(committed.min(pending)),
            (committed, pending) => committed.or(pending),
        }
    });
    let mut watermark_found = watermark.is_none();
    for (number, path) in paths {
        let bytes = fs::read(path).map_err(|error| failure(path, error))?;
        spool_bytes += bytes.len() as u64;
        let mut prefix = bytes.as_slice();
        if let Some((committed_segment, committed_bytes)) = watermark {
            if *number > committed_segment {
                continue;
            }
            if *number == committed_segment {
                let end = usize::try_from(committed_bytes)
                    .ok()
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| {
                        failure(path, "failure witness watermark exceeds the spool length")
                    })?;
                prefix = &bytes[..end];
                watermark_found = true;
            }
        }
        let recovered = recover_prefix(prefix);
        if watermark.is_some_and(|(segment, _)| segment == *number)
            && recovered.valid_bytes != prefix.len()
        {
            return Err(failure(
                path,
                "spool damage precedes the committed failure watermark",
            ));
        }
        if let Some(reason) = &recovered.damage {
            eprintln!(
                "{}: recovered {} valid bytes; {reason}",
                path.display(),
                recovered.valid_bytes
            );
        }
        segments.push((*number, recovered));
    }
    if !watermark_found {
        return Err(failure(
            campaign_dir,
            "failure witness committed segment is missing",
        ));
    }
    for record in segments.iter().flat_map(|(_, recovery)| &recovery.records) {
        if let Record::SessionStart(session) = record {
            if witness.is_some_and(|witness| {
                session.id != witness.session
                    || session.process_start_time != witness.process_start_time
            }) {
                return Err(failure(
                    campaign_dir,
                    "failure witness does not match the spool process session",
                ));
            }
            let context = (
                session.id,
                session.process_id,
                session.process_start_time,
                session.qpc_frequency,
            );
            if process_context.is_some_and(|previous| previous != context) {
                return Err(failure(
                    campaign_dir,
                    "campaign spools in one session directory describe different game processes",
                ));
            }
            *process_context = Some(context);
        }
    }
    let id = campaign_id(&segments).map_err(|error| failure(campaign_dir, error))?;
    let name = campaign_filename(&id).map_err(|error| failure(campaign_dir, error))?;
    if campaign_dir.file_name().and_then(|name| name.to_str()) != Some(name) {
        return Err(failure(
            campaign_dir,
            format!("directory differs from recorded campaign {name}"),
        ));
    }
    if let Some(witness) = witness {
        apply_failure_witness(&mut segments, witness, failing_campaign)
            .map_err(|error| failure(campaign_dir, error))?;
    }
    let destination = replay_dir.join(format!("{name}.battleplan"));
    let mut unchanged = false;
    let mut summary = Summary::default();
    let observe = |frame: &Frame, decoded: &DecodedFrame, sizes| {
        summary
            .observe_decoded(frame, decoded, sizes)
            .map_err(std::io::Error::other)
    };
    let replay = if destination.exists() {
        let previous = open(&destination)?;
        if previous.campaign_id != id {
            return Err(failure(
                &destination,
                "existing replay belongs to a different campaign",
            ));
        }
        if already_finalized(&previous, &segments)? {
            unchanged = true;
            previous
        } else {
            previous
                .append_recovered_with(&segments, observe)
                .map_err(|error| failure(&destination, error))?
        }
    } else {
        Replay::from_recovered_with(id, &segments, observe)
            .map_err(|error| failure(&destination, error))?
    };
    if !unchanged {
        replay
            .write_atomic(&destination)
            .map_err(|error| failure(&destination, error))?;
    }
    let ready_seconds = started.elapsed().as_secs_f64();
    println!(
        "{}{}",
        destination.display(),
        if unchanged {
            " (already finalized; unchanged)"
        } else {
            ""
        }
    );
    println!(
        "finalize_s={ready_seconds:.3} replay_bytes={} spool_bytes={spool_bytes}",
        replay.bytes().len()
    );

    // New captures were summarized during validation and encoding. A repeated
    // finalization skipped those steps, so only that case needs a reporting pass.
    let records: Vec<_> = segments
        .iter()
        .flat_map(|(_, recovered)| &recovered.records)
        .collect();
    if unchanged {
        for record in &records {
            if let Record::Frame(frame) = record {
                summary.observe(frame)?;
            }
        }
    }
    summary.print(spool_bytes);
    print_throughput(
        &replay.sessions,
        records.iter().filter_map(|record| match record {
            Record::Control(control) => Some(Some(control)),
            Record::Gap(_) | Record::BranchStart(_) => Some(None),
            _ => None,
        }),
    );
    let gap_count = records
        .iter()
        .filter(|record| matches!(record, Record::Gap(_)))
        .count();
    println!(
        "gaps={gap_count} damaged_segments={}",
        segments
            .iter()
            .filter(|(_, recovered)| recovered.damage.is_some())
            .count()
    );
    for record in records {
        if let Record::Control(ControlRecord::GapSummary {
            session,
            dropped_frames,
            reason,
        }) = record
        {
            println!(
                "gap_summary session={session} dropped_frames={dropped_frames} hour_range=unknown: {reason}"
            );
        }
    }
    Ok(destination)
}

fn apply_failure_witness(
    segments: &mut [(u32, Recovery)],
    witness: &TerminalWitness,
    failing_campaign: bool,
) -> Result<()> {
    let mut current_branch = None;
    let mut current_signal = None;
    let mut current_observed = false;
    let mut matching_session = false;
    for record in segments.iter().flat_map(|(_, recovery)| &recovery.records) {
        match record {
            Record::SessionStart(session) => {
                if session.id != witness.session
                    || session.process_start_time != witness.process_start_time
                {
                    return Err("failure witness does not match the spool session".into());
                }
                matching_session = true;
            }
            Record::BranchStart(branch) => {
                if branch.session != witness.session {
                    return Err("failure witness does not match the spool branch session".into());
                }
                if current_branch.map(|previous: &replay::branches::BranchStart| previous.id)
                    != Some(branch.id)
                {
                    current_signal = None;
                    current_observed = false;
                }
                current_branch = Some(branch);
            }
            Record::Frame(_) | Record::Gap(_) => current_observed = true,
            Record::Control(ControlRecord::ClockWitness {
                session,
                load_signal,
                ..
            }) => {
                if *session != witness.session {
                    return Err("failure witness does not match the spool clock session".into());
                }
                current_signal = *load_signal;
                current_observed = true;
            }
            _ => {}
        }
    }
    if !matching_session {
        return Err("failure witness has no recorded session identity".into());
    }
    let mut resolved_branch = witness.branch;
    let recovered_branch = if failing_campaign
        && let Some(failed_hour) = witness.failed_hour
        && witness
            .branch
            .is_none_or(|branch| current_branch.map(|current| current.id) != Some(branch))
    {
        let history = Replay::from_recovered(witness.campaign_id.unwrap().to_vec(), segments)
            .and_then(Replay::into_history)
            .map_err(|error| error.to_string())?;
        let session = history
            .sessions
            .iter()
            .find(|session| session.id == witness.session)
            .ok_or("failure witness has no matching history session")?;
        let mut tracker = history
            .tracker_for(session)
            .map_err(|error| error.to_string())?;
        let known_gaps: Vec<_> = segments
            .iter()
            .flat_map(|(_, recovery)| &recovery.records)
            .filter_map(|record| match record {
                Record::Gap(gap)
                    if Some(gap.branch) == tracker.current_branch()
                        && gap.last_hour < failed_hour =>
                {
                    Some(gap.clone())
                }
                _ => None,
            })
            .collect();
        let observation = tracker
            .observe(None, failed_hour, witness.load_signal, &known_gaps)
            .map_err(|error| error.to_string())?;
        if witness
            .branch
            .is_some_and(|expected| expected != observation.branch)
        {
            return Err("failure witness branch differs from the witnessed clock context".into());
        }
        resolved_branch = Some(observation.branch);
        current_signal = witness.load_signal.or(current_signal);
        observation.start
    } else {
        None
    };
    if let Some(branch) = recovered_branch.as_ref() {
        current_branch = Some(branch);
    }
    let mut gap = None;
    if failing_campaign {
        if let Some(expected) = witness.branch
            && current_branch.map(|branch| branch.id) != Some(expected)
        {
            return Err("failure witness branch has no matching committed context".into());
        }
        if let Some(expected) = witness.load_signal
            && current_signal != Some(expected)
        {
            // A context record can commit before the new branch's first clock.
            let first_clock_failed = !current_observed
                && current_signal.is_none()
                && current_branch.is_some_and(|branch| {
                    witness.branch == Some(branch.id)
                        && witness.failed_hour == Some(branch.first_hour)
                });
            if !first_clock_failed {
                return Err("failure witness load signal has no matching committed context".into());
            }
        }
        if let Some(hour) = witness.failed_hour {
            let branch = current_branch
                .filter(|branch| Some(branch.id) == resolved_branch)
                .ok_or("known failure hour has no unambiguous recorded branch context")?;
            if hour < branch.first_hour {
                return Err("failure hour precedes the recorded branch context".into());
            }
            let mut already_gapped = false;
            for record in segments.iter().flat_map(|(_, recovery)| &recovery.records) {
                match record {
                    Record::Frame(frame) if frame.branch == branch.id && frame.hour >= hour => {
                        return Err("failure hour overlaps or precedes a committed frame".into());
                    }
                    Record::Gap(existing)
                        if existing.branch == branch.id
                            && existing.first_hour <= hour
                            && existing.last_hour >= hour =>
                    {
                        if existing.reason != ALLOCATION_GAP_REASON {
                            return Err(
                                "failure hour already belongs to a gap with a different reason"
                                    .into(),
                            );
                        }
                        already_gapped = true;
                    }
                    _ => {}
                }
            }
            if !already_gapped {
                gap = Some(Gap {
                    branch: branch.id,
                    first_hour: hour,
                    last_hour: hour,
                    reason: ALLOCATION_GAP_REASON.into(),
                });
            }
        }
    }
    let partial = ControlRecord::Partial {
        session: witness.session,
        reason: ALLOCATION_GAP_REASON.into(),
        failed_hour: if failing_campaign {
            witness.failed_hour
        } else {
            None
        },
    };
    let mut partial_exists = false;
    for control in segments
        .iter()
        .flat_map(|(_, recovery)| &recovery.records)
        .filter_map(|record| {
            if let Record::Control(control @ ControlRecord::Partial { .. }) = record {
                Some(control)
            } else {
                None
            }
        })
    {
        if control != &partial {
            return Err("recorded partial outcome differs from the failure witness".into());
        }
        partial_exists = true;
    }
    let target = segments
        .iter_mut()
        .rev()
        .find(|(_, recovery)| !recovery.records.is_empty())
        .ok_or("failure witness has no recoverable record prefix")?;
    let mut position = target
        .1
        .records
        .iter()
        .position(|record| matches!(record, Record::Control(ControlRecord::Detach { .. })))
        .unwrap_or(target.1.records.len());
    if let Some(branch) = recovered_branch {
        target
            .1
            .records
            .insert(position, Record::BranchStart(branch));
        position += 1;
        if let Some(Record::Control(ControlRecord::Detach { last_hour, .. })) =
            target.1.records.get_mut(position)
        {
            *last_hour = None;
        }
    }
    if let Some(gap) = gap {
        target.1.records.insert(position, Record::Gap(gap));
        position += 1;
    }
    if !partial_exists {
        target.1.records.insert(position, Record::Control(partial));
    }
    Ok(())
}

fn open(path: &Path) -> Result<Replay> {
    Replay::open(fs::read(path).map_err(|error| failure(path, error))?)
        .map_err(|error| failure(path, error))
}

/// A repeated request is harmless only when its complete recovered contents
/// already exist. Matching hours alone must never hide different row values.
fn already_finalized(previous: &Replay, segments: &[(u32, Recovery)]) -> Result<bool> {
    let records: Vec<_> = segments
        .iter()
        .flat_map(|(_, recovery)| &recovery.records)
        .collect();
    let conflict = || {
        "spool overlaps a finalized session but differs from it; refusing to append duplicate or conflicting hours".to_string()
    };
    let mut expected = BTreeMap::new();
    for record in &records {
        if let Record::Frame(frame) = record
            && expected.insert((frame.branch, frame.hour), frame).is_some()
        {
            return Err(conflict());
        }
    }
    let mut matched = 0;
    for index in &previous.index {
        if expected
            .range((index.branch, index.first_hour)..=(index.branch, index.last_hour))
            .next()
            .is_none()
        {
            continue;
        }
        for frame in previous
            .read_block(index)
            .map_err(|error| error.to_string())?
        {
            if let Some(expected_frame) = expected.remove(&(frame.branch, frame.hour)) {
                if expected_frame != &frame {
                    return Err(conflict());
                }
                matched += 1;
            }
        }
    }
    if matched == 0 {
        return Ok(false);
    }
    if !expected.is_empty() {
        return Err(conflict());
    }
    for record in &records {
        match record {
            Record::SessionStart(session) if !previous.sessions.contains(session) => {
                return Err(conflict());
            }
            Record::BranchStart(branch) if !previous.branches.contains(branch) => {
                return Err(conflict());
            }
            Record::Gap(gap) if !previous.gaps.contains(gap) => return Err(conflict()),
            _ => {}
        }
    }
    let controls = records.iter().filter_map(|record| match record {
        Record::Control(control) => Some(control),
        _ => None,
    });
    if !is_subsequence(previous.controls.iter(), controls) {
        return Err(conflict());
    }
    for (number, recovery) in segments {
        if let Some(reason) = &recovery.damage {
            let session = recovery.records.first().and_then(|record| match record {
                Record::SessionStart(session) => Some(session.id),
                Record::BranchStart(branch) => Some(branch.session),
                _ => None,
            });
            if !previous.recovery_notes.contains(&RecoveryNote {
                session,
                segment: *number,
                valid_bytes: recovery.valid_bytes as u64,
                reason: reason.clone(),
            }) {
                return Err(conflict());
            }
        }
    }
    Ok(true)
}

pub fn inspect(path: &Path) -> Result<()> {
    let replay = open(path)?;
    println!(
        "replay={} bytes={} campaign={:?} keyframe_interval={}",
        path.display(),
        replay.bytes().len(),
        String::from_utf8_lossy(&replay.campaign_id),
        replay.keyframe_interval
    );
    print_identity("header", &replay.identity);
    println!(
        "content_verified={} sessions={} branches={} blocks={}",
        replay.content_verified(),
        replay.sessions.len(),
        replay.branches.len(),
        replay.index.len()
    );
    for session in &replay.sessions {
        println!(
            "session={} pid={} process_start_filetime={} qpc_frequency={} sample_boundary={}",
            session.id,
            session.process_id,
            session.process_start_time,
            session.qpc_frequency,
            session.sample_boundary
        );
        print_identity(&format!("session {}", session.id), &session.identity);
    }
    let mut summary = Summary::default();
    for index in &replay.index {
        println!(
            "block branch={} hours={}..={} offset={} compressed_bytes={} raw_bytes={}",
            index.branch,
            index.first_hour,
            index.last_hour,
            index.offset,
            index.length,
            index.raw_length
        );
        for frame in replay
            .read_block(index)
            .map_err(|error| failure(path, error))?
        {
            summary.observe(&frame)?;
        }
    }
    for branch in &replay.branches {
        let parent = match branch.parent {
            Parentage::Unknown => "unknown".into(),
            Parentage::Probable { branch, hour } => format!(
                "probable({branch}, {hour}); supporting hour {}",
                if summary
                    .hours
                    .get(&branch)
                    .is_some_and(|hours| hours.contains(&hour))
                {
                    "present"
                } else {
                    "missing from recovered replay"
                }
            ),
        };
        println!(
            "branch={} session={} loaded_hour={:?} first_hour={} reason={:?} parent={parent}",
            branch.id, branch.session, branch.loaded_hour, branch.first_hour, branch.reason
        );
    }
    for gap in &replay.gaps {
        println!(
            "gap branch={} hours={}..={}: {}",
            gap.branch, gap.first_hour, gap.last_hour, gap.reason
        );
    }
    for branch in &replay.branches {
        if let Some(hours) = summary.hours.get(&branch.id) {
            for (first, last) in missing_ranges(branch.first_hour, hours) {
                println!(
                    "unrecorded branch={} hours={first}..={last}; see gap and recovery records for available reasons",
                    branch.id
                );
            }
        }
    }
    for note in &replay.recovery_notes {
        println!(
            "recovery session={:?} segment={} valid_bytes={}: {}",
            note.session, note.segment, note.valid_bytes, note.reason
        );
    }
    for control in &replay.controls {
        match control {
            ControlRecord::Detach { session, last_hour } => {
                println!("detach session={session} last_hour={last_hour:?}")
            }
            ControlRecord::GapSummary {
                session,
                dropped_frames,
                reason,
            } => println!(
                "gap_summary session={session} dropped_frames={dropped_frames} hour_range=unknown: {reason}"
            ),
            ControlRecord::Partial {
                session,
                reason,
                failed_hour,
            } => {
                println!("partial session={session} failed_hour={failed_hour:?}: {reason}")
            }
            _ => {}
        }
    }
    summary.print(0);
    for issues in &summary.issues {
        for issue in issues {
            println!(
                "table_gap branch={} table={:?} hours={}..={}: {}",
                issue.branch, issue.table, issue.first, issue.last, issue.reason
            );
        }
    }
    print_throughput(&replay.sessions, replay.controls.iter().map(Some));
    println!(
        "oracle=not run; this command inspects recorded data without asserting save agreement"
    );
    Ok(())
}

/// `recorder inspect <replay> --characters <substring> --hour <h> --branch <id>`:
/// the recorded Characters rows at that frame whose token or name contains
/// the substring, with their template columns.
pub fn inspect_characters(path: &Path, needle: &str, hour: Hour, branch: BranchId) -> Result<()> {
    let replay = open(path)?;
    let frame = match replay
        .seek(branch, hour)
        .map_err(|error| failure(path, error))?
    {
        SeekResult::Frame(frame) => frame,
        SeekResult::Gap { reason, .. } => {
            return Err(format!("branch {branch}, hour {hour} is a gap: {reason}"));
        }
    };
    let lines = character_lines(&frame, needle.as_bytes())?;
    println!(
        "characters branch={branch} hour={hour} matching {needle:?}: {}",
        lines.len()
    );
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

/// One line per Characters row whose token or name contains `needle`, in id
/// order. `template_tag` is shown with its literal from the frame's
/// LexicalTokens when that table is complete and holds the token.
fn character_lines(frame: &DecodedFrame, needle: &[u8]) -> Result<Vec<String>> {
    if needle.is_empty() {
        return Err("--characters needs a nonempty substring".into());
    }
    let contains = |haystack: &[u8]| haystack.windows(needle.len()).any(|part| part == needle);
    let rows = frame
        .complete::<tables::Characters>()
        .map_err(|error| error.to_string())?;
    let literals = frame
        .complete::<tables::LexicalTokens>()
        .map(|rows| {
            rows.iter()
                .map(|row| (row.token, row.literal.as_slice()))
                .collect::<BTreeMap<_, _>>()
        })
        .map_err(|error| error.to_string());
    Ok(rows
        .iter()
        .filter(|row| contains(&row.token) || contains(&row.name))
        .map(|row| {
            let literal = match &literals {
                Ok(literals) => match literals.get(&row.template_tag) {
                    Some(literal) => format!("{:?}", String::from_utf8_lossy(literal)),
                    None => "not in LexicalTokens".into(),
                },
                Err(error) => format!("LexicalTokens unreadable: {error}"),
            };
            format!(
                "character id={} country={} token={:?} name={:?} template_present={} template_tag={} ({literal}) template_file={:?} template_line={} template_generated={}",
                row.id,
                row.country,
                String::from_utf8_lossy(&row.token),
                String::from_utf8_lossy(&row.name),
                row.template_present,
                row.template_tag,
                String::from_utf8_lossy(&row.template_file),
                row.template_line,
                row.template_generated
            )
        })
        .collect())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(out, "{byte:02x}").unwrap();
    }
    out
}

fn is_subsequence<'a, T: PartialEq + 'a>(
    mut existing: impl Iterator<Item = &'a T>,
    incoming: impl Iterator<Item = &'a T>,
) -> bool {
    incoming
        .into_iter()
        .all(|wanted| existing.any(|value| value == wanted))
}

fn missing_ranges(first_hour: Hour, hours: &BTreeSet<Hour>) -> Vec<(Hour, Hour)> {
    let mut missing = Vec::new();
    let mut next = i64::from(first_hour);
    for &hour in hours {
        if i64::from(hour) > next {
            missing.push((next as Hour, hour - 1));
        }
        next = i64::from(hour) + 1;
    }
    missing
}

fn print_identity(label: &str, identity: &Identity) {
    println!(
        "{label}: game={:?} exe_sha256={} exe_bytes={} vanilla_sha256={} language={:?}",
        identity.game_version,
        hex(&identity.exe_hash),
        identity.exe_size,
        hex(&identity.vanilla_hash),
        identity.language
    );
    println!(
        "{label}: coverage_policy={} recorder={:?} collector_sha256={}",
        identity.coverage_policy,
        identity.recorder_version,
        hex(&identity.collector_hash)
    );
    if identity.coverage_policy == replay::identity::COVERAGE_POLICY_VERSION {
        println!(
            "coverage: descriptor plus map, common, history, events, localisation, interface, gfx/flags, gfx/interface, gfx/texticons, gfx/leaders and every .asset and .gfx descriptor below gfx; integrated DLC and Tweaker UI roots are included in the vanilla hash under their install-relative paths; music, sound, video, loading screens, launcher, crash reporter and logs excluded"
        );
    } else {
        println!("coverage: this recorder does not recognize the policy; no coverage claim");
    }
    for (position, layer) in identity.dlc.iter().enumerate() {
        println!(
            "dlc[{position}] id={:?} sha256={} declared_checksum={:?}",
            layer.id,
            hex(&layer.hash),
            layer.declared_checksum
        );
    }
    for (position, layer) in identity.mods.iter().enumerate() {
        println!(
            "mod[{position}] id={:?} name={:?} declared_version={:?} sha256={}",
            layer.id,
            layer.display_name,
            layer.declared_version,
            hex(&layer.hash)
        );
    }
    match &identity.verification {
        Verification::Unchecked => println!("{label}: verification=unchecked"),
        Verification::Verified { launcher, memory } => {
            println!("{label}: verification=verified launcher={launcher:?} memory={memory:?}")
        }
        Verification::Unverified {
            reason,
            launcher,
            memory,
        } => println!(
            "{label}: verification=unverified reason={reason:?} launcher={launcher:?} memory={memory:?}"
        ),
    }
}

#[derive(Debug)]
struct Issue {
    branch: BranchId,
    table: TableId,
    first: Hour,
    last: Hour,
    reason: String,
}

struct Summary {
    decoder: FrameDecoder,
    totals: Vec<u64>,
    table_times: [Vec<u64>; TABLE_COUNT],
    table_bytes: [u64; TABLE_COUNT],
    states: [[u64; 5]; TABLE_COUNT],
    max_pool: u64,
    max_dropped: u64,
    hours: BTreeMap<BranchId, BTreeSet<Hour>>,
    issues: [Vec<Issue>; TABLE_COUNT],
}

impl Default for Summary {
    fn default() -> Self {
        Self {
            decoder: FrameDecoder::default(),
            totals: Vec::new(),
            table_times: std::array::from_fn(|_| Vec::new()),
            table_bytes: [0; TABLE_COUNT],
            states: [[0; 5]; TABLE_COUNT],
            max_pool: 0,
            max_dropped: 0,
            hours: BTreeMap::new(),
            issues: std::array::from_fn(|_| Vec::new()),
        }
    }
}

impl Summary {
    fn observe(&mut self, frame: &Frame) -> Result<()> {
        let decoded = self
            .decoder
            .apply(frame)
            .map_err(|error| error.to_string())?;
        let sizes = frame
            .table_encoded_sizes()
            .map_err(|error| error.to_string())?;
        self.observe_decoded(frame, &decoded, sizes)
    }

    fn observe_decoded(
        &mut self,
        frame: &Frame,
        decoded: &DecodedFrame,
        sizes: [usize; TABLE_COUNT],
    ) -> Result<()> {
        self.totals.push(frame.metrics.total_ns);
        self.max_pool = self.max_pool.max(frame.metrics.pool_bytes);
        self.max_dropped = self.max_dropped.max(frame.metrics.dropped_frames);
        if !self
            .hours
            .entry(frame.branch)
            .or_default()
            .insert(frame.hour)
        {
            return Err(format!(
                "duplicate frame on branch {} at hour {}",
                frame.branch, frame.hour
            ));
        }
        for table in TableId::ALL {
            let i = table.index();
            self.table_bytes[i] += sizes[i] as u64;
            self.table_times[i].push(frame.metrics.table_ns[i]);
            let state = match &frame.tables[i] {
                TableUpdate::Snapshot(_) => 0,
                TableUpdate::Delta { .. } => 1,
                TableUpdate::Unchanged { .. } => 2,
                TableUpdate::Truncated { .. } => 3,
                TableUpdate::Unavailable { .. } => 4,
            };
            self.states[i][state] += 1;
            let reason = match &*decoded.tables[i] {
                TableView::Complete(_) => continue,
                TableView::Truncated { missing, .. } => {
                    format!("truncated; {missing} rows not read")
                }
                TableView::Unavailable { reason } => format!("unavailable; {reason}"),
            };
            if let Some(previous) = self.issues[i].last_mut()
                && previous.branch == frame.branch
                && previous.last.checked_add(1) == Some(frame.hour)
                && previous.reason == reason
            {
                previous.last = frame.hour;
            } else {
                self.issues[i].push(Issue {
                    branch: frame.branch,
                    table,
                    first: frame.hour,
                    last: frame.hour,
                    reason,
                });
            }
        }
        Ok(())
    }

    fn print(&mut self, spool_bytes: u64) {
        let hours = self.totals.len();
        let [p50, p95, maximum] = percentiles(&mut self.totals);
        println!(
            "hours={hours} total_p50_ms={p50:.3} total_p95_ms={p95:.3} total_max_ms={maximum:.3} pool_max_bytes={} dropped_counter_max={}",
            self.max_pool, self.max_dropped
        );
        if spool_bytes != 0 && hours != 0 {
            println!(
                "spool_bytes_per_hour={:.3}",
                spool_bytes as f64 / hours as f64
            );
        }
        println!(
            "table raw_encoded_bytes bytes_per_hour read_p50_ms read_p95_ms read_max_ms snapshots deltas unchanged truncated unavailable"
        );
        for table in TableId::ALL {
            let i = table.index();
            let [p50, p95, maximum] = percentiles(&mut self.table_times[i]);
            let [snapshots, deltas, unchanged, truncated, unavailable] = self.states[i];
            let per_hour = if hours == 0 {
                0.0
            } else {
                self.table_bytes[i] as f64 / hours as f64
            };
            println!(
                "{table:?} {} {per_hour:.3} {p50:.3} {p95:.3} {maximum:.3} {snapshots} {deltas} {unchanged} {truncated} {unavailable}",
                self.table_bytes[i]
            );
        }
    }
}

fn percentiles(values: &mut [u64]) -> [f64; 3] {
    if values.is_empty() {
        return [0.0; 3];
    }
    values.sort_unstable();
    let p50 = values[values.len().div_ceil(2) - 1];
    let p95 = values[(values.len() * 95).div_ceil(100) - 1];
    [p50, p95, *values.last().unwrap()].map(|ns| ns as f64 / 1_000_000.0)
}

#[derive(Default)]
struct Throughput {
    previous: BTreeMap<SessionId, (u64, Hour, Option<i64>)>,
    hours: u64,
    seconds: f64,
}

impl Throughput {
    fn witness(
        &mut self,
        session: SessionId,
        frequency: u64,
        qpc: u64,
        hour: Hour,
        signal: Option<i64>,
    ) {
        let previous = self.previous.insert(session, (qpc, hour, signal));
        if frequency != 0
            && let Some((last_qpc, last_hour, last_signal)) = previous
            && last_hour.checked_add(1) == Some(hour)
            && signal == last_signal
            && qpc > last_qpc
        {
            self.hours += 1;
            self.seconds += (qpc - last_qpc) as f64 / frequency as f64;
        }
    }
}

// A None marks a branch or gap boundary in a recovered spool's record stream.
fn print_throughput<'a>(
    sessions: &[SessionStart],
    controls: impl Iterator<Item = Option<&'a ControlRecord>>,
) {
    let frequencies: BTreeMap<_, _> = sessions
        .iter()
        .map(|session| (session.id, session.qpc_frequency))
        .collect();
    let mut throughput = Throughput::default();
    for control in controls {
        match control {
            Some(ControlRecord::ClockWitness {
                session,
                qpc,
                hour,
                load_signal,
            }) => {
                throughput.witness(
                    *session,
                    frequencies.get(session).copied().unwrap_or(0),
                    *qpc,
                    *hour,
                    *load_signal,
                );
            }
            Some(
                ControlRecord::Detach { session, .. }
                | ControlRecord::GapSummary { session, .. }
                | ControlRecord::Partial { session, .. },
            ) => {
                throughput.previous.remove(session);
            }
            None => throughput.previous.clear(),
        }
    }
    if throughput.seconds > 0.0 {
        println!(
            "consecutive_witness_hours={} witness_seconds={:.3} game_hours_per_second={:.3}",
            throughput.hours,
            throughput.seconds,
            throughput.hours as f64 / throughput.seconds
        );
    } else {
        println!(
            "game_hours_per_second=unavailable (no consecutive clock witnesses with a valid frequency)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::branches::{BranchReason, BranchStart};

    #[test]
    fn allocation_failure_json_configuration_requires_prepared_identity() {
        use crate::test_support::{allocation_records, allocation_test_directory};
        let directory = allocation_test_directory("prepared-identity");
        let Record::SessionStart(session) = &allocation_records()[0] else {
            unreachable!()
        };
        let mut session_start = Vec::new();
        session.encode(&mut session_start).unwrap();
        let config = serde_json::json!({"session_id":session.id,"session_start":session_start});
        fs::create_dir_all(directory.join("bin")).unwrap();
        fs::write(
            directory.join("bin/collector.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let path = directory.join(WITNESS_FILE_NAME);
        fs::write(&path, TerminalWitness::prepared(7, 13)).unwrap();
        assert!(read_failure_witness(&directory).unwrap().is_none());
        fs::write(&path, TerminalWitness::prepared(7, 14)).unwrap();
        assert!(read_failure_witness(&directory).is_err());
        fs::write(&path, [0; WITNESS_BYTES]).unwrap();
        assert!(read_failure_witness(&directory).is_err());
    }

    #[test]
    #[ignore = "launched by collector allocation-failure sweeps"]
    fn allocation_fixture_finalization_child() {
        let session = PathBuf::from(std::env::var_os("BATTLE_PLAN_ALLOCATION_FIXTURE").or_else(|| std::env::var_os("HOI4_ALLOCATION_FIXTURE")).unwrap());
        let witness = read_failure_witness(&session).unwrap().unwrap();
        let mut expected = BTreeMap::new();
        let mut sources = Vec::new();
        let mut refused = BTreeSet::new();
        for campaign in fs::read_dir(&session).unwrap() {
            let campaign = campaign.unwrap();
            if !campaign.file_type().unwrap().is_dir() {
                continue;
            }
            for (number, path) in segment_paths(&campaign.path()).unwrap() {
                let bytes = fs::read(&path).unwrap();
                let mut prefix = bytes.as_slice();
                let committed_campaign = witness.committed_campaign_id.is_some_and(|id| {
                    campaign.file_name().to_str().map(str::as_bytes) == Some(id.as_slice())
                });
                let pending_campaign = witness.pending_campaign_id.is_some_and(|id| {
                    campaign.file_name().to_str().map(str::as_bytes) == Some(id.as_slice())
                });
                if witness.committed_campaign_id.is_none()
                    || (pending_campaign && witness.pending_segment.is_none())
                {
                    refused.insert(campaign.file_name());
                    prefix = &[];
                } else {
                    let committed = committed_campaign
                        .then(|| (witness.committed_segment.unwrap(), witness.committed_bytes));
                    let pending = pending_campaign
                        .then(|| (witness.pending_segment.unwrap(), witness.pending_bytes));
                    if let Some((segment, committed_bytes)) =
                        [committed, pending].into_iter().flatten().min()
                    {
                        if number > segment {
                            prefix = &[];
                        } else if number == segment {
                            prefix = &bytes[..usize::try_from(committed_bytes).unwrap()];
                        }
                    }
                }
                let recovery = recover_prefix(prefix);
                for record in recovery.records {
                    if let Record::Frame(frame) = record {
                        assert!(
                            expected
                                .insert((campaign.file_name(), frame.branch, frame.hour), frame)
                                .is_none()
                        );
                    }
                }
                sources.push((path, bytes));
            }
        }
        let output = session.join("replays");
        let (paths, issues) = finalize_session_outcome(&session, &output).unwrap();
        assert_eq!(issues.len(), refused.len(), "{issues:?}");
        assert!(
            issues
                .iter()
                .all(|issue| issue.contains("no committed spool watermark"))
        );
        assert_eq!(paths.is_empty(), expected.is_empty());
        let mut gap_found = false;
        let mut gap_expected = false;
        let mut allocation_gap_count = 0;
        let mut before = Vec::new();
        for path in &paths {
            let bytes = fs::read(path).unwrap();
            let replay = Replay::open(bytes.clone()).unwrap();
            let expected_failed_hour = if witness.campaign_id.as_ref().map(|id| id.as_slice())
                == Some(replay.campaign_id.as_slice())
            {
                witness.failed_hour
            } else {
                None
            };
            gap_expected |= expected_failed_hour.is_some();
            allocation_gap_count += replay
                .gaps
                .iter()
                .filter(|gap| gap.reason == ALLOCATION_GAP_REASON)
                .count();
            assert!(replay.controls.iter().any(|control| matches!(control,
                ControlRecord::Partial { session, reason, failed_hour }
                if *session == witness.session && reason == ALLOCATION_GAP_REASON
                    && *failed_hour == expected_failed_hour)));
            if let Some(detach) = replay
                .controls
                .iter()
                .position(|control| matches!(control, ControlRecord::Detach { .. }))
            {
                let partial = replay
                    .controls
                    .iter()
                    .position(|control| matches!(control, ControlRecord::Partial { .. }))
                    .unwrap();
                assert!(partial < detach);
            }
            // Finalization's SegmentState rejects a gap after Detach.
            for block in &replay.index {
                let frames = replay.read_block(block).unwrap();
                let mut decoder = FrameDecoder::new();
                for frame in frames {
                    let key = (
                        std::ffi::OsString::from(std::str::from_utf8(&replay.campaign_id).unwrap()),
                        frame.branch,
                        frame.hour,
                    );
                    let expected_frame = expected
                        .remove(&key)
                        .expect("replay contains an unrecorded frame");
                    assert_eq!(frame, expected_frame);
                    decoder.apply(&frame).unwrap();
                    assert!(matches!(
                        replay.seek(frame.branch, frame.hour).unwrap(),
                        SeekResult::Frame(_)
                    ));
                }
            }
            if let Some(hour) = expected_failed_hour {
                let gap = replay
                    .gaps
                    .iter()
                    .find(|gap| {
                        gap.first_hour <= hour
                            && gap.last_hour >= hour
                            && gap.reason == ALLOCATION_GAP_REASON
                    })
                    .expect("known attempted hour has no explicit allocation gap");
                assert!(
                    matches!(replay.seek(gap.branch, hour).unwrap(), SeekResult::Gap { reason, .. } if reason == ALLOCATION_GAP_REASON)
                );
                gap_found = true;
            } else {
                assert!(
                    !replay
                        .gaps
                        .iter()
                        .any(|gap| gap.reason == ALLOCATION_GAP_REASON)
                );
            }
            before.push(bytes);
        }
        assert!(expected.is_empty(), "committed prefix frames were lost");
        assert_eq!(gap_found, gap_expected);
        assert_eq!(allocation_gap_count, usize::from(gap_expected));
        let (again, repeated_issues) = finalize_session_outcome(&session, &output).unwrap();
        assert_eq!(repeated_issues, issues);
        assert_eq!(again, paths);
        for (path, bytes) in paths.iter().zip(before) {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        for (path, bytes) in sources {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        assert_eq!(read_failure_witness(&session).unwrap(), Some(witness));
    }

    fn allocation_session(name: &str) -> (PathBuf, PathBuf, TerminalWitness) {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_records, allocation_test_directory, allocation_witness,
        };
        let root = allocation_test_directory(name);
        let session = root.join("session");
        let campaign = session.join(std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap());
        let output = root.join("replays");
        fs::create_dir_all(&campaign).unwrap();
        let mut spool = Vec::new();
        for record in allocation_records() {
            spool.extend(replay::spool::encode_record(&record).unwrap());
        }
        let witness = allocation_witness(spool.len() as u64);
        // A writer can leave a torn record after the last committed watermark.
        spool.extend_from_slice(b"torn write beyond the committed prefix");
        fs::write(campaign.join("spool.0000.h4s"), spool).unwrap();
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        (session, output, witness)
    }

    #[test]
    fn allocation_failure_finalizes_verified_prefix_with_exact_gap_and_is_idempotent() {
        let (session, output, witness) = allocation_session("partial-prefix");
        let paths = finalize_session(&session, &output).unwrap();
        assert_eq!(paths.len(), 1);
        let bytes = fs::read(&paths[0]).unwrap();
        let replay = Replay::open(bytes.clone()).unwrap();
        assert_eq!(
            replay
                .index
                .iter()
                .map(|index| replay.read_block(index).unwrap().len())
                .sum::<usize>(),
            3
        );
        assert_eq!(
            replay.gaps,
            [Gap {
                branch: 9,
                first_hour: 99,
                last_hour: 99,
                reason: ALLOCATION_GAP_REASON.into(),
            }]
        );
        assert!(
            matches!(replay.seek(9, 99).unwrap(), SeekResult::Gap { reason, .. } if reason == ALLOCATION_GAP_REASON)
        );
        let partial = replay.controls.iter().position(|control| matches!(control,
            ControlRecord::Partial { session: 7, failed_hour: Some(99), reason } if reason.contains(ALLOCATION_GAP_REASON))).unwrap();
        let detach = replay
            .controls
            .iter()
            .position(|control| matches!(control, ControlRecord::Detach { .. }))
            .unwrap();
        assert!(partial < detach);
        assert_eq!(finalize_session(&session, &output).unwrap(), paths);
        assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
        assert!(session.join(WITNESS_FILE_NAME).is_file());
        let campaign = session.join(std::str::from_utf8(&witness.campaign_id.unwrap()).unwrap());
        assert!(campaign.join("spool.0000.h4s").is_file());
    }

    #[test]
    fn allocation_failure_unknown_hour_does_not_invent_a_tail_gap() {
        let (session, output, mut witness) = allocation_session("partial-idle");
        witness.failed_hour = None;
        witness.drain_complete = true;
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert!(replay.gaps.is_empty());
        assert!(replay.controls.iter().any(|control| matches!(
            control,
            ControlRecord::Partial {
                failed_hour: None,
                ..
            }
        )));
    }

    #[test]
    fn allocation_failure_crc_valid_record_after_watermark_is_not_committed() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("uncommitted-frame");
        let mut records = allocation_records();
        records.pop();
        let mut spool = Vec::new();
        for record in &records {
            spool.extend(replay::spool::encode_record(record).unwrap());
        }
        witness.committed_bytes = spool.len() as u64;
        witness.failed_hour = Some(5);
        let mut frame = records
            .iter()
            .rev()
            .find_map(|record| match record {
                Record::Frame(frame) => Some(frame.clone()),
                _ => None,
            })
            .unwrap();
        frame.hour = 5;
        let TableUpdate::Snapshot(TableRows::Clock(rows)) =
            &mut frame.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0].hour = 5;
        spool.extend(replay::spool::encode_record(&Record::Frame(frame)).unwrap());
        assert_eq!(recover_prefix(&spool).valid_bytes, spool.len());
        let campaign = session.join(std::str::from_utf8(&witness.campaign_id.unwrap()).unwrap());
        let path = campaign.join("spool.0000.h4s");
        fs::write(&path, &spool).unwrap();
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert!(matches!(replay.seek(9, 4).unwrap(), SeekResult::Frame(_)));
        assert!(matches!(replay.seek(9, 5).unwrap(), SeekResult::Gap { .. }));
        assert_eq!(fs::read(path).unwrap(), spool);
    }

    #[test]
    fn allocation_failure_callback_without_spool_branch_resolves_existing_load_context() {
        let (session, output, mut witness) = allocation_session("callback-branch");
        witness.branch = None;
        witness.failed_hour = Some(5);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert_eq!(replay.branches.len(), 1);
        assert_eq!(
            replay.gaps,
            [Gap {
                branch: 9,
                first_hour: 5,
                last_hour: 5,
                reason: ALLOCATION_GAP_REASON.into(),
            }]
        );
    }

    #[test]
    fn allocation_failure_declared_branch_before_initial_clock_preserves_exact_prefix() {
        use crate::test_support::allocation_records;
        for (name, signal, failed_hour, recoverable) in [
            ("declared-no-clock", None, 21, true),
            ("declared-matching-clock", Some(Some(21)), 21, true),
            ("declared-wrong-hour", None, 22, false),
            ("declared-wrong-clock", Some(Some(22)), 21, false),
            ("declared-unknown-clock", Some(None), 21, false),
        ] {
            let (session, output, mut witness) = allocation_session(name);
            let mut records = allocation_records();
            records.pop();
            let expected_frames: Vec<_> = records
                .iter()
                .filter_map(|record| match record {
                    Record::Frame(frame) => Some(frame.clone()),
                    _ => None,
                })
                .collect();
            let branch = BranchStart {
                id: 10,
                session: witness.session,
                loaded_hour: Some(20),
                first_hour: 21,
                parent: Parentage::Unknown,
                reason: BranchReason::LoadSignal,
            };
            records.push(Record::BranchStart(branch.clone()));
            if let Some(load_signal) = signal {
                records.push(Record::Control(ControlRecord::ClockWitness {
                    session: witness.session,
                    qpc: 2100,
                    hour: 21,
                    load_signal,
                }));
            }
            let mut spool = Vec::new();
            for record in &records {
                spool.extend(replay::spool::encode_record(record).unwrap());
            }
            assert_eq!(recover_prefix(&spool).valid_bytes, spool.len());
            witness.committed_bytes = spool.len() as u64;
            witness.branch = Some(branch.id);
            witness.failed_hour = Some(failed_hour);
            witness.load_signal = Some(21);
            spool.extend_from_slice(b"torn bytes after declared branch");
            let campaign =
                session.join(std::str::from_utf8(&witness.campaign_id.unwrap()).unwrap());
            let spool_path = campaign.join("spool.0000.h4s");
            let witness_path = session.join(WITNESS_FILE_NAME);
            fs::write(&spool_path, &spool).unwrap();
            fs::write(&witness_path, witness.encode()).unwrap();

            let result = finalize_session(&session, &output);
            if recoverable {
                let paths = result.unwrap();
                let bytes = fs::read(&paths[0]).unwrap();
                let replay = Replay::open(bytes.clone()).unwrap();
                assert_eq!(replay.branches[1], branch);
                assert_eq!(replay.branches.len(), 2);
                let frames: Vec<_> = replay
                    .index
                    .iter()
                    .flat_map(|index| replay.read_block(index).unwrap())
                    .collect();
                assert_eq!(frames, expected_frames);
                assert_eq!(
                    replay.gaps,
                    [Gap {
                        branch: 10,
                        first_hour: 21,
                        last_hour: 21,
                        reason: ALLOCATION_GAP_REASON.into(),
                    }]
                );
                assert!(matches!(
                    replay.seek(10, 21).unwrap(),
                    SeekResult::Gap { .. }
                ));
                assert_eq!(
                    replay
                        .controls
                        .iter()
                        .filter(|control| matches!(
                            control,
                            ControlRecord::ClockWitness { hour: 21, .. }
                        ))
                        .count(),
                    usize::from(signal.is_some())
                );
                assert!(replay.controls.iter().any(|control| matches!(control,
                    ControlRecord::Partial { failed_hour: Some(21), reason, .. }
                        if reason == ALLOCATION_GAP_REASON)));
                assert_eq!(finalize_session(&session, &output).unwrap(), paths);
                assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .contains("load signal has no matching committed context")
                );
                assert!(fs::read_dir(&output).unwrap().next().is_none());
            }
            assert_eq!(fs::read(&spool_path).unwrap(), spool);
            assert_eq!(fs::read(&witness_path).unwrap(), witness.encode());
        }
    }

    #[test]
    fn allocation_failure_known_hour_without_campaign_keeps_prefix_without_a_gap() {
        let (session, output, mut witness) = allocation_session("unknown-attempted-campaign");
        witness.campaign_id = None;
        witness.branch = None;
        witness.load_signal = Some(77);
        witness.failed_hour = Some(21);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert!(matches!(replay.seek(9, 4).unwrap(), SeekResult::Frame(_)));
        assert_eq!(replay.branches.len(), 1);
        assert!(replay.gaps.is_empty());
        assert!(replay.controls.iter().any(|control| matches!(
            control,
            ControlRecord::Partial {
                failed_hour: None,
                ..
            }
        )));
        assert_eq!(read_failure_witness(&session).unwrap(), Some(witness));
    }

    #[test]
    fn allocation_failure_same_signal_hour_discontinuity_uses_native_branch_semantics() {
        let (session, output, mut witness) = allocation_session("callback-hour-discontinuity");
        witness.branch = None;
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let before = fs::read(&paths[0]).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert_eq!(replay.branches[1].reason, BranchReason::HourDiscontinuity);
        assert_eq!(replay.branches[1].loaded_hour, None);
        assert_eq!(replay.branches[1].parent, Parentage::Unknown);
        assert_eq!(
            replay.gaps,
            [Gap {
                branch: 10,
                first_hour: 99,
                last_hour: 99,
                reason: ALLOCATION_GAP_REASON.into()
            }]
        );
        assert_eq!(finalize_session(&session, &output).unwrap(), paths);
        assert_eq!(fs::read(&paths[0]).unwrap(), before);
    }

    #[test]
    fn allocation_failure_unknown_load_read_retains_last_native_signal() {
        let (session, output, mut witness) = allocation_session("callback-unknown-load-read");
        witness.branch = None;
        witness.load_signal = None;
        witness.failed_hour = Some(5);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert_eq!(replay.branches.len(), 1);
        assert_eq!(
            replay.gaps,
            [Gap {
                branch: 9,
                first_hour: 5,
                last_hour: 5,
                reason: ALLOCATION_GAP_REASON.into()
            }]
        );
    }

    #[test]
    fn allocation_failure_existing_gaps_preserve_native_branch_continuity() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("callback-known-gaps");
        let mut records = allocation_records();
        records.insert(
            records.len() - 1,
            Record::Gap(Gap {
                branch: 9,
                first_hour: 5,
                last_hour: 98,
                reason: "synthetic witnessed missing hours".into(),
            }),
        );
        let mut bytes = Vec::new();
        for record in records {
            bytes.extend(replay::spool::encode_record(&record).unwrap());
        }
        let campaign = session.join(std::str::from_utf8(&witness.campaign_id.unwrap()).unwrap());
        witness.branch = None;
        witness.committed_bytes = bytes.len() as u64;
        fs::write(campaign.join("spool.0000.h4s"), bytes).unwrap();
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert_eq!(replay.branches.len(), 1);
        assert_eq!(replay.gaps.len(), 2);
        assert_eq!(
            replay.gaps[1],
            Gap {
                branch: 9,
                first_hour: 99,
                last_hour: 99,
                reason: ALLOCATION_GAP_REASON.into(),
            }
        );
    }

    #[test]
    fn allocation_failure_changed_load_signal_recovers_a_gap_only_branch_idempotently() {
        let (session, output, mut witness) = allocation_session("callback-load-failure");
        witness.branch = None;
        witness.load_signal = Some(20);
        witness.failed_hour = Some(1);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let bytes = fs::read(&paths[0]).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert_eq!(replay.branches.len(), 2);
        assert_eq!(
            replay.branches[1],
            BranchStart {
                id: 10,
                session: 7,
                loaded_hour: None,
                first_hour: 1,
                parent: Parentage::Unknown,
                reason: BranchReason::LoadSignal,
            }
        );
        assert_eq!(
            replay.gaps,
            [Gap {
                branch: 10,
                first_hour: 1,
                last_hour: 1,
                reason: ALLOCATION_GAP_REASON.into(),
            }]
        );
        assert!(
            matches!(replay.seek(10, 1).unwrap(), SeekResult::Gap { reason, .. } if reason == ALLOCATION_GAP_REASON)
        );
        assert!(matches!(
            replay.controls.last().unwrap(),
            ControlRecord::Detach {
                last_hour: None,
                ..
            }
        ));
        assert_eq!(finalize_session(&session, &output).unwrap(), paths);
        assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
    }

    #[test]
    fn allocation_failure_new_campaign_without_prefix_preserves_previous_campaign() {
        let (session, output, mut witness) = allocation_session("new-campaign-failure");
        witness.campaign_id = Some(*b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6");
        witness.branch = None;
        witness.load_signal = Some(20);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert!(replay.gaps.is_empty());
        assert!(replay.controls.iter().any(|control| matches!(
            control,
            ControlRecord::Partial {
                failed_hour: None,
                ..
            }
        )));
    }

    #[test]
    fn allocation_failure_mixed_campaign_completion_keeps_successful_outputs_in_result() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("mixed-campaign-failure");
        witness.campaign_id = Some(*b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6");
        witness.pending_campaign_id = witness.campaign_id;
        witness.branch = None;
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let failed = session.join(std::str::from_utf8(&witness.campaign_id.unwrap()).unwrap());
        fs::create_dir_all(&failed).unwrap();
        let context = allocation_records();
        let mut spool = Vec::new();
        for record in &context[..2] {
            spool.extend(replay::spool::encode_record(record).unwrap());
        }
        fs::write(failed.join("spool.0000.h4s"), spool).unwrap();
        let (paths, issues) = finalize_session_outcome(&session, &output).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("no committed spool watermark"));
        assert!(
            open(&paths[0])
                .unwrap()
                .controls
                .iter()
                .any(|control| matches!(
                    control,
                    ControlRecord::Partial {
                        failed_hour: None,
                        ..
                    }
                ))
        );
    }

    #[test]
    fn allocation_failure_new_campaign_caps_late_crc_valid_previous_campaign_frame() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("mixed-late-frame");
        let committed = witness.committed_campaign_id.unwrap();
        let old = session.join(std::str::from_utf8(&committed).unwrap());
        let mut records = allocation_records();
        records.pop();
        let mut old_spool = Vec::new();
        for record in &records {
            old_spool.extend(replay::spool::encode_record(record).unwrap());
        }
        witness.committed_bytes = old_spool.len() as u64;
        let mut late = records
            .iter()
            .rev()
            .find_map(|record| match record {
                Record::Frame(frame) => Some(frame.clone()),
                _ => None,
            })
            .unwrap();
        late.hour = 5;
        let TableUpdate::Snapshot(TableRows::Clock(rows)) =
            &mut late.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0].hour = 5;
        old_spool.extend(replay::spool::encode_record(&Record::Frame(late.clone())).unwrap());
        assert_eq!(recover_prefix(&old_spool).valid_bytes, old_spool.len());
        let old_path = old.join("spool.0000.h4s");
        fs::write(&old_path, &old_spool).unwrap();

        let attempted = *b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
        witness.campaign_id = Some(attempted);
        witness.pending_campaign_id = Some(attempted);
        witness.branch = None;
        witness.load_signal = Some(20);
        witness.failed_hour = Some(99);
        let failed = session.join(std::str::from_utf8(&attempted).unwrap());
        fs::create_dir_all(&failed).unwrap();
        let mut failed_spool = replay::spool::encode_record(&records[0]).unwrap();
        failed_spool.extend(
            replay::spool::encode_record(&Record::BranchStart(BranchStart {
                id: 10,
                session: 7,
                loaded_hour: Some(98),
                first_hour: 99,
                parent: Parentage::Unknown,
                reason: BranchReason::LoadSignal,
            }))
            .unwrap(),
        );
        late.branch = 10;
        late.hour = 99;
        let TableUpdate::Snapshot(TableRows::Clock(rows)) =
            &mut late.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0].hour = 99;
        rows[0].play_session_started_at = 20;
        let TableUpdate::Snapshot(TableRows::Campaign(rows)) =
            &mut late.tables[TableId::Campaign.index()]
        else {
            unreachable!()
        };
        rows[0].id = attempted.to_vec();
        failed_spool.extend(replay::spool::encode_record(&Record::Frame(late)).unwrap());
        assert_eq!(
            recover_prefix(&failed_spool).valid_bytes,
            failed_spool.len()
        );
        let failed_path = failed.join("spool.0000.h4s");
        fs::write(&failed_path, &failed_spool).unwrap();
        let witness_bytes = witness.encode();
        fs::write(session.join(WITNESS_FILE_NAME), witness_bytes).unwrap();

        let (paths, issues) = finalize_session_outcome(&session, &output).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("no committed spool watermark"));
        let replay = open(&paths[0]).unwrap();
        assert_eq!(replay.campaign_id, committed);
        assert!(replay.index.iter().all(|block| block.last_hour <= 4));
        assert!(matches!(replay.seek(9, 4).unwrap(), SeekResult::Frame(_)));
        assert!(replay.gaps.is_empty());
        assert!(replay.controls.iter().any(|control| matches!(
            control,
            ControlRecord::Partial {
                failed_hour: None,
                ..
            }
        )));
        assert!(
            !output
                .join(format!(
                    "{}.battleplan",
                    std::str::from_utf8(&attempted).unwrap()
                ))
                .exists()
        );
        let before = fs::read(&paths[0]).unwrap();
        assert_eq!(
            finalize_session_outcome(&session, &output).unwrap(),
            (paths.clone(), issues)
        );
        assert_eq!(fs::read(&paths[0]).unwrap(), before);
        assert_eq!(fs::read(old_path).unwrap(), old_spool);
        assert_eq!(fs::read(failed_path).unwrap(), failed_spool);
        assert_eq!(
            fs::read(session.join(WITNESS_FILE_NAME)).unwrap(),
            witness_bytes
        );
    }

    #[test]
    fn allocation_failure_without_global_watermark_refuses_raw_crc_valid_frames() {
        let (session, output, mut witness) = allocation_session("uncertified-prefix");
        witness.campaign_id = None;
        witness.branch = None;
        witness.load_signal = None;
        witness.failed_hour = None;
        witness.committed_campaign_id = None;
        witness.committed_segment = None;
        witness.committed_bytes = 0;
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let (paths, issues) = finalize_session_outcome(&session, &output).unwrap();
        assert!(paths.is_empty());
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("no committed spool watermark"));
        assert!(fs::read_dir(output).unwrap().next().is_none());
    }

    #[test]
    fn allocation_failure_pending_old_campaign_caps_late_frame_with_other_global_watermark() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("pending-other-campaign");
        let pending = witness.committed_campaign_id.unwrap();
        let mut old_records = allocation_records();
        old_records.pop();
        let mut old_spool = Vec::new();
        for record in &old_records {
            old_spool.extend(replay::spool::encode_record(record).unwrap());
        }
        witness.pending_campaign_id = Some(pending);
        witness.pending_segment = Some(0);
        witness.pending_bytes = old_spool.len() as u64;
        let mut late = old_records
            .iter()
            .rev()
            .find_map(|record| match record {
                Record::Frame(frame) => Some(frame.clone()),
                _ => None,
            })
            .unwrap();
        late.hour = 5;
        let TableUpdate::Snapshot(TableRows::Clock(rows)) =
            &mut late.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0].hour = 5;
        old_spool.extend(replay::spool::encode_record(&Record::Frame(late)).unwrap());
        assert_eq!(recover_prefix(&old_spool).valid_bytes, old_spool.len());
        let old_path = session
            .join(std::str::from_utf8(&pending).unwrap())
            .join("spool.0000.h4s");
        fs::write(&old_path, &old_spool).unwrap();

        let committed = *b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
        let mut current_spool = Vec::new();
        for mut record in old_records {
            if let Record::Frame(frame) = &mut record {
                let TableUpdate::Snapshot(TableRows::Campaign(rows)) =
                    &mut frame.tables[TableId::Campaign.index()]
                else {
                    unreachable!()
                };
                rows[0].id = committed.to_vec();
            }
            current_spool.extend(replay::spool::encode_record(&record).unwrap());
        }
        let current = session.join(std::str::from_utf8(&committed).unwrap());
        fs::create_dir_all(&current).unwrap();
        let current_path = current.join("spool.0000.h4s");
        fs::write(&current_path, &current_spool).unwrap();
        witness.campaign_id = Some(committed);
        witness.committed_campaign_id = Some(committed);
        witness.committed_bytes = current_spool.len() as u64;
        let witness_bytes = witness.encode();
        fs::write(session.join(WITNESS_FILE_NAME), witness_bytes).unwrap();

        let (paths, issues) = finalize_session_outcome(&session, &output).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(paths.len(), 2);
        let mut before = Vec::new();
        for path in &paths {
            let replay = open(path).unwrap();
            assert!(replay.index.iter().all(|block| block.last_hour <= 4));
            assert!(matches!(replay.seek(9, 4).unwrap(), SeekResult::Frame(_)));
            let failed_hour = if replay.campaign_id == committed {
                Some(99)
            } else {
                None
            };
            assert!(replay.controls.iter().any(|control| matches!(control,
                ControlRecord::Partial { failed_hour: actual, .. } if *actual == failed_hour)));
            assert_eq!(replay.gaps.len(), usize::from(failed_hour.is_some()));
            before.push(fs::read(path).unwrap());
        }
        assert_eq!(
            finalize_session_outcome(&session, &output).unwrap(),
            (paths.clone(), Vec::new())
        );
        for (path, bytes) in paths.iter().zip(before) {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        assert_eq!(fs::read(old_path).unwrap(), old_spool);
        assert_eq!(fs::read(current_path).unwrap(), current_spool);
        assert_eq!(
            fs::read(session.join(WITNESS_FILE_NAME)).unwrap(),
            witness_bytes
        );
    }

    #[test]
    fn allocation_failure_stable_failed_campaign_keeps_prefix_with_other_global_watermark() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("stable-failed-campaign");
        let committed = *b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
        let mut spool = Vec::new();
        for mut record in allocation_records() {
            if let Record::Frame(frame) = &mut record {
                let TableUpdate::Snapshot(TableRows::Campaign(rows)) =
                    &mut frame.tables[TableId::Campaign.index()]
                else {
                    unreachable!()
                };
                rows[0].id = committed.to_vec();
            }
            spool.extend(replay::spool::encode_record(&record).unwrap());
        }
        let current = session.join(std::str::from_utf8(&committed).unwrap());
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join("spool.0000.h4s"), &spool).unwrap();
        witness.committed_campaign_id = Some(committed);
        witness.committed_bytes = spool.len() as u64;
        witness.failed_hour = Some(5);
        witness.branch = None;
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let (paths, issues) = finalize_session_outcome(&session, &output).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(paths.len(), 2);
        for path in paths {
            let replay = open(&path).unwrap();
            assert!(matches!(replay.seek(9, 4).unwrap(), SeekResult::Frame(_)));
            assert_eq!(
                replay.gaps.len(),
                usize::from(replay.campaign_id != committed)
            );
            if replay.campaign_id != committed {
                assert!(
                    matches!(replay.seek(9, 5).unwrap(), SeekResult::Gap { reason, .. } if reason == ALLOCATION_GAP_REASON)
                );
            }
        }
    }

    #[test]
    fn allocation_failure_same_campaign_pending_cap_uses_earlier_watermark() {
        use crate::test_support::allocation_records;
        let (session, output, mut witness) = allocation_session("pending-earlier-watermark");
        let mut prior = Vec::new();
        for record in allocation_records() {
            if matches!(&record, Record::Frame(frame) if frame.hour == 4) {
                break;
            }
            prior.extend(replay::spool::encode_record(&record).unwrap());
        }
        witness.pending_campaign_id = witness.committed_campaign_id;
        witness.pending_segment = Some(0);
        witness.pending_bytes = prior.len() as u64;
        witness.failed_hour = Some(4);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let replay = open(&paths[0]).unwrap();
        assert!(replay.index.iter().all(|block| block.last_hour <= 3));
        assert!(matches!(replay.seek(9, 3).unwrap(), SeekResult::Frame(_)));
        assert!(
            matches!(replay.seek(9, 4).unwrap(), SeekResult::Gap { reason, .. } if reason == ALLOCATION_GAP_REASON)
        );
    }

    #[test]
    fn allocation_failure_reuses_an_existing_gap_before_detach() {
        use crate::test_support::{allocation_records, allocation_witness};
        let mut records = allocation_records();
        let gap = Gap {
            branch: 9,
            first_hour: 99,
            last_hour: 99,
            reason: ALLOCATION_GAP_REASON.into(),
        };
        records.insert(records.len() - 1, Record::Gap(gap));
        let mut segments = vec![(
            0,
            Recovery {
                records,
                valid_bytes: 1,
                damage: None,
            },
        )];
        let witness = allocation_witness(1);
        apply_failure_witness(&mut segments, &witness, true).unwrap();
        apply_failure_witness(&mut segments, &witness, true).unwrap();
        let records = &segments[0].1.records;
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record, Record::Gap(_)))
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record, Record::Control(ControlRecord::Partial { .. })))
                .count(),
            1
        );
        assert!(matches!(
            records[records.len() - 1],
            Record::Control(ControlRecord::Detach { .. })
        ));
    }

    #[test]
    fn allocation_failure_refuses_invalid_identity_context_and_watermarks() {
        let (session, output, witness) = allocation_session("partial-refusals");
        for bad in [
            TerminalWitness {
                session: 8,
                ..witness
            },
            TerminalWitness {
                process_start_time: 14,
                ..witness
            },
            TerminalWitness {
                branch: Some(100),
                ..witness
            },
            TerminalWitness {
                load_signal: Some(20),
                ..witness
            },
            TerminalWitness {
                failed_hour: Some(4),
                ..witness
            },
            TerminalWitness {
                committed_segment: Some(1),
                ..witness
            },
            TerminalWitness {
                committed_bytes: witness.committed_bytes - 1,
                ..witness
            },
            TerminalWitness {
                committed_bytes: u64::MAX,
                ..witness
            },
        ] {
            fs::write(session.join(WITNESS_FILE_NAME), bad.encode()).unwrap();
            assert!(finalize_session(&session, &output).is_err(), "{bad:?}");
            assert!(fs::read_dir(&output).unwrap().next().is_none());
        }
        let mut torn = witness.encode();
        torn[31] ^= 1;
        fs::write(session.join(WITNESS_FILE_NAME), torn).unwrap();
        assert!(finalize_session(&session, &output).is_err());
        fs::write(session.join(WITNESS_FILE_NAME), &torn[..WITNESS_BYTES - 1]).unwrap();
        assert!(finalize_session(&session, &output).is_err());
        fs::remove_file(session.join(WITNESS_FILE_NAME)).unwrap();
        fs::write(session.join("status.json"), br#"{"capture_failed":true}"#).unwrap();
        assert!(finalize_session(&session, &output).is_err());
    }

    #[test]
    fn allocation_failure_before_first_frame_retains_evidence_without_a_replay() {
        use crate::test_support::{allocation_test_directory, allocation_witness};
        let session = allocation_test_directory("bootstrap-failure");
        let witness = TerminalWitness {
            campaign_id: None,
            branch: None,
            load_signal: None,
            failed_hour: None,
            committed_campaign_id: None,
            committed_segment: None,
            committed_bytes: 0,
            ..allocation_witness(0)
        };
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let output = session.join("output");
        assert!(
            finalize_session(&session, &output)
                .unwrap_err()
                .contains("no campaign spool")
        );
        assert!(read_failure_witness(&session).unwrap().is_some());
    }

    #[test]
    fn allocation_failure_empty_witness_preserves_healthy_finalization_bytes() {
        let (session, output, witness) = allocation_session("healthy-witness");
        let campaign = session.join(std::str::from_utf8(&witness.campaign_id.unwrap()).unwrap());
        fs::OpenOptions::new()
            .write(true)
            .open(campaign.join("spool.0000.h4s"))
            .unwrap()
            .set_len(witness.committed_bytes)
            .unwrap();
        fs::remove_file(session.join(WITNESS_FILE_NAME)).unwrap();
        let paths = finalize_session(&session, &output).unwrap();
        let before = fs::read(&paths[0]).unwrap();
        fs::write(session.join(WITNESS_FILE_NAME), [0; WITNESS_BYTES]).unwrap();
        assert_eq!(finalize_session(&session, &output).unwrap(), paths);
        assert_eq!(fs::read(&paths[0]).unwrap(), before);
        let replay = Replay::open(before).unwrap();
        assert!(replay.gaps.is_empty());
        assert!(
            !replay
                .controls
                .iter()
                .any(|control| matches!(control, ControlRecord::Partial { .. }))
        );
    }

    #[test]
    fn character_lines_filter_by_token_or_name_and_resolve_the_template_tag() {
        use replay::{
            frame::Metrics,
            rows::{CharacterRow, LexicalTokenRow},
        };
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        let row = |id, token: &[u8], name: &[u8], tag, line| CharacterRow {
            id,
            token: token.to_vec(),
            name: name.to_vec(),
            template_present: 1,
            template_tag: tag,
            template_file: b"common/characters/ARG.txt".to_vec(),
            template_line: line,
            ..Default::default()
        };
        *frame.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(vec![
                row(1, b"ARG_pedro_pablo_ramirez", b"", 7, 263),
                row(2, b"ARG_other", b"ARG_pedro_pablo_ramirez", 8, 329),
                row(3, b"GER_other", b"", 7, 1),
            ]));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 7,
                literal: b"ARG_pedro_pablo_ramirez".to_vec(),
            }]));
        let lines = character_lines(&frame, b"pedro_pablo").unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("character id=1 "));
        assert!(lines[0].contains(
            "template_tag=7 (\"ARG_pedro_pablo_ramirez\") template_file=\"common/characters/ARG.txt\" template_line=263"
        ));
        assert!(lines[1].contains("template_tag=8 (not in LexicalTokens)"));
        assert!(character_lines(&frame, b"").is_err());
        *frame.tables[TableId::Characters.index()] = TableView::Unavailable {
            reason: "test".into(),
        };
        assert!(character_lines(&frame, b"pedro").is_err());
    }

    #[test]
    fn file_names_are_validated_without_rewriting_campaign_identity() {
        let mixed_case = "21b7CC84-3028-4f65-8E0d-aDf2f37Ca1a6";
        assert_eq!(
            campaign_filename(mixed_case.as_bytes()).unwrap(),
            mixed_case
        );
        assert_eq!(
            campaign_filename(b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6").unwrap(),
            "21b7cc84-3028-4f65-8e0d-adf2f37ca1a6"
        );
        for id in [
            b"../campaign".as_slice(),
            b"21b7cc84/3028-4f65-8e0d-adf2f37ca1a6",
            b"21b7cc84-3028-4f65-8e0d-adf2f37ca1ag",
        ] {
            assert!(campaign_filename(id).is_err());
        }
        assert_eq!(segment_number("spool.10000.h4s").unwrap(), Some(10000));
        assert_eq!(segment_number("spool.0009.h4s").unwrap(), Some(9));
        assert_eq!(segment_number("status.json").unwrap(), None);
        assert!(segment_number("spool.-001.h4s").is_err());
    }

    #[test]
    fn percentiles_use_nearest_rank_without_rounding_native_samples() {
        let mut samples: Vec<_> = (1..=100).rev().map(|n| n * 1_000_000).collect();
        assert_eq!(percentiles(&mut samples), [50.0, 95.0, 100.0]);
        assert_eq!(percentiles(&mut [1]), [0.000001; 3]);
        assert_eq!(percentiles(&mut []), [0.0; 3]);
    }

    #[test]
    fn throughput_excludes_reload_gaps_and_process_boundaries() {
        let mut result = Throughput::default();
        result.witness(1, 100, 100, 50, Some(1));
        result.witness(1, 100, 120, 51, Some(1));
        result.witness(1, 100, 150, 52, Some(2)); // next-hour reload
        result.witness(1, 100, 160, 70, Some(2)); // unexplained jump
        result.witness(1, 100, 180, 71, Some(2));
        result.witness(2, 100, 200, 72, Some(2)); // new process
        result.witness(2, 100, 190, 73, Some(2)); // invalid QPC ordering
        result.witness(3, 0, 100, 1, None);
        result.witness(3, 0, 200, 2, None);
        assert_eq!(result.hours, 2);
        assert!((result.seconds - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn repeated_controls_require_order_and_multiplicity() {
        assert!(is_subsequence([1, 2, 2, 3].iter(), [2, 2].iter()));
        assert!(!is_subsequence([1, 2, 3].iter(), [2, 2].iter()));
        assert!(!is_subsequence([1, 2, 3].iter(), [3, 2].iter()));
    }

    #[test]
    fn repeat_finalization_compares_values_and_allows_a_fresh_continuation() {
        use replay::{
            branches::{BranchReason, BranchStart},
            frame::Metrics,
            rows::{CampaignRow, ClockRow, SunkShipsRow, WorldTensionRow},
        };
        let id = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6".to_vec();
        let session = Record::SessionStart(SessionStart {
            id: 1,
            process_id: 2,
            process_start_time: 3,
            qpc_frequency: 100,
            sample_boundary: replay::identity::SampleBoundary::EndOfHour,
            identity: Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: vec![],
                mods: vec![],
                coverage_policy: 1,
                language: "english".into(),
                recorder_version: "test".into(),
                collector_hash: [3; 32],
                verification: Verification::Unchecked,
            },
        });
        let branch = Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            loaded_hour: Some(1),
            first_hour: 2,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        });
        let frame = |hour, seed| {
            Record::Frame(Frame {
                branch: 1,
                hour,
                keyframe: true,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|index| match TableId::ALL[index] {
                    TableId::Clock => TableUpdate::Snapshot(TableRows::Clock(vec![ClockRow {
                        hour,
                        seed,
                        ..Default::default()
                    }])),
                    TableId::Campaign => {
                        TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
                            id: id.clone(),
                        }]))
                    }
                    TableId::SunkShips => {
                        TableUpdate::Snapshot(TableRows::SunkShips(vec![SunkShipsRow::default()]))
                    }
                    TableId::WorldTension => TableUpdate::Snapshot(TableRows::WorldTension(vec![
                        WorldTensionRow::default(),
                    ])),
                    TableId::LocalPlayer => {
                        TableUpdate::Snapshot(TableRows::LocalPlayer(vec![Default::default()]))
                    }
                    TableId::SimulationFeatures => {
                        TableUpdate::Snapshot(TableRows::SimulationFeatures(vec![
                            Default::default(),
                        ]))
                    }
                    TableId::CombatLogState => {
                        TableUpdate::Snapshot(TableRows::CombatLogState(vec![Default::default()]))
                    }
                    TableId::AirSortieModifierMetadata => {
                        TableUpdate::Snapshot(TableRows::AirSortieModifierMetadata(vec![
                            Default::default(),
                        ]))
                    }
                    _ => TableUpdate::Snapshot(TableRows::empty(TableId::ALL[index])),
                }),
            })
        };
        let witness = Record::Control(ControlRecord::ClockWitness {
            session: 1,
            qpc: 200,
            hour: 2,
            load_signal: None,
        });
        let first = vec![
            session.clone(),
            branch.clone(),
            frame(2, 10),
            witness.clone(),
        ];
        let previous = Replay::from_records(id.clone(), std::slice::from_ref(&first)).unwrap();
        let recovered = |records| {
            vec![(
                0,
                Recovery {
                    records,
                    valid_bytes: 0,
                    damage: None,
                },
            )]
        };
        assert!(already_finalized(&previous, &recovered(first)).unwrap());
        assert!(
            already_finalized(
                &previous,
                &recovered(vec![
                    session.clone(),
                    branch.clone(),
                    frame(2, 10),
                    witness.clone(),
                    witness,
                ])
            )
            .is_err()
        );
        assert!(
            already_finalized(
                &previous,
                &recovered(vec![session.clone(), branch.clone(), frame(2, 11)])
            )
            .is_err()
        );
        let continuation = recovered(vec![session.clone(), branch.clone(), frame(3, 10)]);
        assert!(!already_finalized(&previous, &continuation).unwrap());
        assert!(previous.append_recovered(&continuation).is_ok());
        assert!(
            already_finalized(
                &previous,
                &recovered(vec![session, branch, frame(2, 10), frame(3, 10)])
            )
            .is_err()
        );
    }

    #[test]
    fn missing_ranges_use_exact_recorded_hours_and_do_not_invent_a_tail() {
        let hours = [12, 13, 17, 19].into_iter().collect();
        assert_eq!(missing_ranges(10, &hours), [(10, 11), (14, 16), (18, 18)]);
        assert!(missing_ranges(10, &BTreeSet::new()).is_empty());
        assert!(missing_ranges(Hour::MAX, &[Hour::MAX].into_iter().collect()).is_empty());
    }
}
