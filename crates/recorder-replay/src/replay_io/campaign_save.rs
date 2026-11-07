//! Each save of a campaign's spool into its one recording: the records its segments add, every
//! page its captures' files published since the last save of a branch the recording holds, a
//! capture writer's stop, and the session's game facts once, in one revision.

use replay::{
    campaign::{CampaignWriter, CaptureStop, Records, Replay, Revision, records_of, writer},
    columnar_file::capture_of,
    columnar_frames::{Recording, schema},
    spool::{Recovered, Recovery},
};
use replay_columnar::envelope;
use replay_data::{BranchId, Hour, identity::IdentityCodec};
use replay_frame::branches::Gap;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io,
    path::{Path, PathBuf},
};

/// What a save added.
pub(super) struct Saved {
    pub replay: Replay,
    /// Whether it added anything: none adds no revision.
    pub changed: bool,
    pub hours: u64,
    /// The segments with a frame hour the recording neither holds nor names as a gap yet: their
    /// captures publish it later, so they stay unsaved.
    pub unheld: BTreeSet<u32>,
}

/// The capture files in `campaign_dir`, by capture: each one's path and whether it is finished.
pub(super) fn capture_files(campaign_dir: &Path) -> io::Result<Vec<(u32, PathBuf, bool)>> {
    let entries = match fs::read_dir(campaign_dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        entries => entries?,
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        if let Some((capture, partial)) = capture_of(&entry.file_name()) {
            found.push((capture, entry.path(), !partial));
        }
    }
    // A finished file and its partial never stand together; the finished one is taken.
    found.sort_by_key(|&(capture, _, finished)| (capture, !finished));
    found.dedup_by_key(|(capture, _, _)| *capture);
    Ok(found)
}

/// The spool folder `campaign_dir` lies in, as run sources name it.
fn spool_of(campaign_dir: &Path) -> io::Result<String> {
    campaign_dir
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| io::Error::other("a campaign spool lies in no session folder"))
}

/// Adds every page `campaign_dir`'s captures published of the branches `keep` keeps, and each
/// finished capture's writer stop the recording does not hold yet.
fn ingest(
    writer: &mut CampaignWriter<fs::File>,
    campaign_dir: &Path,
    keep: &dyn Fn(BranchId) -> bool,
    stops: &[CaptureStop],
    records: &mut Records,
    unreadable: &mut Vec<String>,
    check: &dyn Fn() -> io::Result<()>,
) -> io::Result<u64> {
    let spool = spool_of(campaign_dir)?;
    let mut hours = 0;
    for (number, path, finished) in capture_files(campaign_dir)? {
        check()?;
        let capture = match Recording::open_path(&path) {
            Ok(capture) => capture,
            // Its writer has not published a revision yet, or never will: [`account`] tells which.
            Err(error) if !finished && error.kind() == io::ErrorKind::InvalidData => {
                unreadable.push(format!("{}: {error}", path.display()));
                continue;
            }
            Err(error) => return Err(io::Error::new(error.kind(), format!("{}: {error}", path.display()))),
        };
        hours += writer
            .ingest(&capture, &spool, number, keep)
            .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?
            .hours;
        if finished
            && let Some(stop) = capture.capture()?.and_then(|capture| capture.stop)
            && keep(stop.branch)
            && !stops
                .iter()
                .chain(&records.stops)
                .any(|held| held.spool == spool && held.capture == number)
        {
            records.stops.push(CaptureStop {
                spool: spool.clone(),
                capture: number,
                branch: stop.branch,
                last_hour: stop.last_hour,
                reason: stop.reason,
            });
        }
    }
    Ok(hours)
}

/// Holds every frame hour of `segments` of a branch `keep` keeps to account against the hours the
/// file holds once `writer` has taken this save's pages. A gap this save adds names only hours the
/// file lacks: a captured hour is recorded, not lost. An hour the file lacks and no gap names is,
/// once the session has `ended`, named by a gap of its own; until then its capture may publish it
/// later, and its segment is returned to stay unsaved.
fn account(
    writer: &CampaignWriter<fs::File>,
    segments: &[(u32, Recovery<Recovered>)],
    keep: &dyn Fn(BranchId) -> bool,
    gaps: &[Gap],
    records: &mut Records,
    ended: bool,
    unreadable: &[String],
) -> io::Result<BTreeSet<u32>> {
    let mut held: BTreeMap<BranchId, Vec<(i64, i64)>> = BTreeMap::new();
    for (branch, first, hours) in writer.runs() {
        if hours > 0 {
            let first = i64::from(first);
            held.entry(branch).or_default().push((first, first + i64::from(hours) - 1));
        }
    }
    let holds = |branch: BranchId, hour: i64| {
        held.get(&branch)
            .is_some_and(|spans| spans.iter().any(|&(first, last)| first <= hour && hour <= last))
    };
    let mut trimmed = Vec::new();
    for gap in std::mem::take(&mut records.gaps) {
        let mut first = i64::from(gap.first_hour);
        let last = i64::from(gap.last_hour);
        while first <= last {
            while first <= last && holds(gap.branch, first) {
                first += 1;
            }
            let mut end = first;
            while end < last && !holds(gap.branch, end + 1) {
                end += 1;
            }
            if first <= last {
                trimmed.push(Gap {
                    branch: gap.branch,
                    first_hour: hour(first)?,
                    last_hour: hour(end)?,
                    reason: engine_allocation::string(&gap.reason)?,
                });
            }
            first = end + 1;
        }
    }
    records.gaps = trimmed;
    let named = |branch: BranchId, hour: i64| {
        gaps.iter().chain(&records.gaps).any(|gap| {
            gap.branch == branch && i64::from(gap.first_hour) <= hour && hour <= i64::from(gap.last_hour)
        })
    };
    let mut unheld = BTreeSet::new();
    let mut lost: BTreeMap<BranchId, BTreeSet<i64>> = BTreeMap::new();
    for (number, recovery) in segments {
        for record in &recovery.records {
            let Recovered::Frame(frame) = record else {
                continue;
            };
            let at = i64::from(frame.hour);
            if !keep(frame.branch) || holds(frame.branch, at) || named(frame.branch, at) {
                continue;
            }
            if ended {
                lost.entry(frame.branch).or_default().insert(at);
            } else {
                unheld.insert(*number);
            }
        }
    }
    let reason = if unreadable.is_empty() {
        "no capture of the session holds these recorded hours".to_owned()
    } else {
        format!(
            "no capture of the session holds these recorded hours; unreadable: {}",
            unreadable.join("; ")
        )
    };
    for (branch, hours) in lost {
        let mut hours = hours.into_iter().peekable();
        while let Some(first) = hours.next() {
            let mut last = first;
            while hours.peek() == Some(&(last + 1)) {
                last = hours.next().expect("peeked");
            }
            engine_allocation::reserve(&mut records.gaps, 1)?;
            records.gaps.push(Gap {
                branch,
                first_hour: hour(first)?,
                last_hour: hour(last)?,
                reason: engine_allocation::string(&reason)?,
            });
        }
    }
    Ok(unheld)
}

fn hour(at: i64) -> io::Result<Hour> {
    Hour::try_from(at).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "an hour passes 2^31"))
}

/// The branches a recording holding `previous` and `records` names: those whose hours it saves.
fn known_branches(previous: Option<&Replay>, records: &Records) -> BTreeSet<BranchId> {
    previous
        .into_iter()
        .flat_map(|replay| replay.branches.iter())
        .chain(&records.branches)
        .map(|branch| branch.id)
        .collect()
}

/// Adds `segments` and `campaign_dir`'s captures to `previous`, the recording at `target` at its
/// revision, as one revision; `facts` once when it carries none. Every frame hour of `retained`,
/// the spool's segments `segments` ends, is accounted for ([`account`]). `prepare` is told the
/// revision before it is published. A failure cuts the file back to `previous`'s revision.
#[allow(clippy::too_many_arguments)]
pub(super) fn append(
    target: &Path,
    previous: &Replay,
    segments: &[(u32, Recovery<Recovered>)],
    retained: &[(u32, Recovery<Recovered>)],
    campaign_dir: &Path,
    facts: Option<&[u8]>,
    level: i32,
    ended: bool,
    check: &dyn Fn() -> io::Result<()>,
    prepare: impl FnOnce(Revision) -> io::Result<()>,
) -> io::Result<Saved> {
    check()?;
    let mut records = records_of(segments, &previous.identity, &previous.campaign_id, Some(previous))?;
    let before = previous.revision();
    let mut unheld = BTreeSet::new();
    let written: io::Result<(Option<Revision>, u64)> = (|| {
        let mut writer = CampaignWriter::resume(previous, target, level)?;
        let known = known_branches(Some(previous), &records);
        let keep = |branch| known.contains(&branch);
        let mut unreadable = Vec::new();
        let hours = ingest(
            &mut writer,
            campaign_dir,
            &keep,
            &previous.stops,
            &mut records,
            &mut unreadable,
            check,
        )?;
        unheld = account(&writer, retained, &keep, &previous.gaps, &mut records, ended, &unreadable)?;
        writer.add_records(&mut records)?;
        if previous.fact_packs.is_empty()
            && let Some(pack) = facts
        {
            writer.add_facts(pack)?;
        }
        if writer.is_empty() {
            return Ok((None, 0));
        }
        check()?;
        let revision = writer.revise_with(|file| file.sync_data(), prepare)?;
        Ok((Some(revision), hours))
    })();
    match written {
        Ok((None, _)) => Ok(Saved {
            replay: Replay::open_path(target)?,
            changed: false,
            hours: 0,
            unheld,
        }),
        Ok((Some(revision), hours)) => {
            let replay = Replay::open_path(target)?;
            if replay.revision() != revision {
                return Err(io::Error::other("the recording changed after it was saved"));
            }
            Ok(Saved {
                replay,
                changed: true,
                hours,
                unheld,
            })
        }
        Err(error) => {
            // Readers keep the revision they opened; the torn tail goes.
            if let Err(rollback) = writer::roll_back(target, before) {
                return Err(io::Error::new(
                    error.kind(),
                    format!("{error}; the recording could not be restored: {rollback}"),
                ));
            }
            Err(error)
        }
    }
}

/// A new recording of campaign `campaign_id` holding `segments` and `campaign_dir`'s captures,
/// and `facts` when given, written to `path`, a new file. None when the segments name no
/// session.
pub(super) fn create(
    path: &Path,
    campaign_id: &[u8],
    segments: &[(u32, Recovery<Recovered>)],
    campaign_dir: &Path,
    facts: impl FnOnce(&replay_data::identity::Identity) -> Option<Vec<u8>>,
    level: i32,
    ended: bool,
    check: &dyn Fn() -> io::Result<()>,
) -> io::Result<Option<Saved>> {
    let Some(session) = segments
        .iter()
        .flat_map(|(_, recovery)| &recovery.records)
        .find_map(|record| match record {
            Recovered::SessionStart(session) => Some(session.clone()),
            _ => None,
        })
    else {
        return Ok(None);
    };
    let mut records = records_of(segments, &session.identity, campaign_id, None)?;
    let mut content = Vec::new();
    session.identity.encode(&mut content)?;
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&schema()?),
        campaign: campaign_id.to_vec(),
        content,
        capture_mode: session.capture_mode.byte(),
    };
    let written = (|| {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        let mut writer = CampaignWriter::create(file, identity, level)?;
        let known = known_branches(None, &records);
        let keep = |branch| known.contains(&branch);
        let mut unreadable = Vec::new();
        let hours = ingest(&mut writer, campaign_dir, &keep, &[], &mut records, &mut unreadable, check)?;
        let unheld = account(&writer, segments, &keep, &[], &mut records, ended, &unreadable)?;
        writer.add_records(&mut records)?;
        if let Some(pack) = facts(&session.identity) {
            writer.add_facts(&pack)?;
        }
        check()?;
        writer.revise(|file| file.sync_data())?;
        writer.into_inner().sync_all()?;
        Ok::<_, io::Error>((hours, unheld))
    })();
    match written {
        Ok((hours, unheld)) => Ok(Some(Saved {
            replay: Replay::open_path(path)?,
            changed: true,
            hours,
            unheld,
        })),
        Err(error) => {
            let _ = fs::remove_file(path);
            Err(error)
        }
    }
}

/// Removes every capture of `campaign_dir`, finished or still written, and every journal a
/// capture would be drained from: discarded hours a later save would otherwise take again.
pub(super) fn remove_captures(campaign_dir: &Path) -> io::Result<()> {
    let entries = match fs::read_dir(campaign_dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        entries => entries?,
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if capture_of(&name).is_some() || replay::columnar_journal::segment_of(&name).is_some() {
            match replay::publication::posix_delete(&entry.path()) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
    }
    Ok(())
}

/// Once the session has `ended`, removes each finished capture of `campaign_dir` whose hours
/// `replay` holds whole: every run of a branch it names copied to its last page; runs of
/// branches it never named are not saved.
pub(super) fn retire_saved_captures(campaign_dir: &Path, replay: &Replay, ended: bool) -> io::Result<usize> {
    // The collector numbers a new capture after the captures it finds: one removed while the
    // session runs would give its number, and its run sources, to the next.
    if !ended {
        return Ok(0);
    }
    let spool = spool_of(campaign_dir)?;
    let known: BTreeSet<BranchId> = replay.branches.iter().map(|branch| branch.id).collect();
    let mut removed = 0;
    for (number, path, finished) in capture_files(campaign_dir)? {
        if !finished {
            continue;
        }
        let whole = {
            let capture = match Recording::open_path(&path) {
                Ok(capture) => capture,
                Err(error) if error.kind() == io::ErrorKind::InvalidData => continue,
                Err(error) => return Err(error),
            };
            let mut whole = capture.damage().is_empty();
            for at in 0..capture.run_count() {
                let Ok(run) = capture.run(at) else {
                    whole = false;
                    break;
                };
                if !known.contains(&run.branch) {
                    continue;
                }
                let held = replay
                    .sources
                    .iter()
                    .find(|source| source.spool == spool && source.capture == number && source.capture_run == at)
                    .and_then(|source| replay.recording().run(source.run).ok());
                whole &= run.pages() == 0
                    || held.is_some_and(|held| held.pages() == run.pages() && held.chain == run.chain);
            }
            whole
        };
        if whole {
            fs::remove_file(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Whether `campaign_dir`'s captures published pages of a branch `previous` names that it does
/// not hold yet.
pub(super) fn pending(campaign_dir: &Path, previous: &Replay) -> io::Result<bool> {
    let spool = spool_of(campaign_dir)?;
    for (number, path, _) in capture_files(campaign_dir)? {
        let Ok(capture) = Recording::open_path(&path) else {
            continue;
        };
        for at in 0..capture.run_count() {
            let Ok(run) = capture.run(at) else {
                continue;
            };
            if run.pages() == 0 || !previous.branches.iter().any(|branch| branch.id == run.branch) {
                continue;
            }
            let held = previous
                .sources
                .iter()
                .find(|source| source.spool == spool && source.capture == number && source.capture_run == at)
                .and_then(|source| previous.recording().run(source.run).ok())
                .map_or(0, |held| held.pages());
            if held < run.pages() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Adds game facts pack `pack` to `replay`, the recording at `path` at its revision, as a revision
/// of its own, when it carries none.
pub(super) fn append_facts(path: &Path, replay: &Replay, pack: &[u8], level: i32) -> io::Result<bool> {
    if !replay.fact_packs.is_empty() {
        return Ok(false);
    }
    let mut writer = CampaignWriter::resume(replay, path, level)?;
    let before = replay.revision();
    let written = writer
        .add_facts(pack)
        .and_then(|()| writer.revise(|file| file.sync_data()));
    if let Err(error) = written {
        let _ = writer::roll_back(path, before);
        return Err(error);
    }
    Ok(true)
}
