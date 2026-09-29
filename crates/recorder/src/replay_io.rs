//! Passive finalization and inspection use the same codecs as the collector.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use replay::{
    BranchId, Hour, SessionId,
    branches::Parentage,
    container::{RecoveryNote, Replay, SeekResult},
    frame::{DecodedFrame, Frame, FrameDecoder, TABLE_COUNT, TableUpdate, TableView},
    identity::{Identity, SessionStart, Verification},
    rows::{TableId, TableRows, tables},
    spool::{ControlRecord, Record, Recovery, recover_prefix},
};

type Result<T> = std::result::Result<T, String>;

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
    if id.len() != 36
        || id.iter().enumerate().any(|(position, byte)| {
            if [8, 13, 18, 23].contains(&position) {
                *byte != b'-'
            } else {
                !byte.is_ascii_hexdigit()
            }
        })
    {
        return Err(
            "recorded campaign identity is not a UUID; refusing to use it as a file name".into(),
        );
    }
    std::str::from_utf8(id).map_err(|error| error.to_string())
}

/// Call only after detach and module-absence confirmation, or after process exit.
pub fn finalize_session(session_dir: &Path, replay_dir: &Path) -> Result<Vec<PathBuf>> {
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
        match finalize_campaign(&campaign_dir, &paths, replay_dir, &mut process_context) {
            Ok(path) => outputs.push(path),
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Ok(outputs)
    } else {
        Err(errors.join("\n"))
    }
}

fn finalize_campaign(
    campaign_dir: &Path,
    paths: &[(u32, PathBuf)],
    replay_dir: &Path,
    process_context: &mut Option<(SessionId, u32, u64, u64)>,
) -> Result<PathBuf> {
    let started = Instant::now();
    let mut spool_bytes = 0_u64;
    let mut segments = Vec::with_capacity(paths.len());
    for (number, path) in paths {
        let bytes = fs::read(path).map_err(|error| failure(path, error))?;
        spool_bytes += bytes.len() as u64;
        let recovered = recover_prefix(&bytes);
        if let Some(reason) = &recovered.damage {
            eprintln!(
                "{}: recovered {} valid bytes; {reason}",
                path.display(),
                recovered.valid_bytes
            );
        }
        segments.push((*number, recovered));
    }
    for record in segments.iter().flat_map(|(_, recovery)| &recovery.records) {
        if let Record::SessionStart(session) = record {
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
            "coverage: descriptor plus map, common, history, localisation, interface, gfx/flags, gfx/interface and every .asset and .gfx descriptor below gfx; integrated DLC and Tweaker UI roots are included in the vanilla hash under their install-relative paths; music, sound, video, loading screens, launcher, crash reporter and logs excluded"
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
                ControlRecord::Detach { session, .. } | ControlRecord::GapSummary { session, .. },
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
