//! Append-only episode chunks, tally runs and bounded observer continuation.
use super::*;
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use dashboard_text::metadata::check_cancel;

mod block;
mod newest;
#[cfg(any(test, feature = "test-support"))]
pub use block::kept_hours;
pub use block::{FAMILY, fold, observe_block};

#[derive(Serialize, Deserialize)]
struct EpisodeChunk {
    ordinal: u64,
    episode: RaidEpisode,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    next_episode: u64,
    ordinals: Vec<u64>,
    events: Accumulator,
    tallies: Accumulator,
    coverage: Accumulator,
    present: bool,
    reads: Accumulator,
    read_known: bool,
    covered: BTreeSet<u64>,
    supplied: BTreeSet<u64>,
    #[serde(skip)]
    keys: BTreeMap<String, u64>,
}
impl Prepared {
    #[cfg(test)]
    pub(super) fn work(&self) -> dashboard_base::prepared::observer_tests::Work {
        self.scan.work
    }
    #[cfg(test)]
    fn observe_with_content(
        &mut self,
        frame: &DecodedFrame,
        format: replay::format::FormatVersion,
    ) -> io::Result<()> {
        let changes = dashboard_base::prepared::frame_changes::FrameChangeTracker::new()
            .observe(frame, format);
        self.observe_changed(frame, format, None, &changes)
    }
    pub fn observe_changed(
        &mut self,
        frame: &DecodedFrame,
        format: replay::format::FormatVersion,
        content: Option<&Content>,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.scan.format = format;
        if !self.present
            && [TableId::RaidCountries, TableId::Raids]
                .iter()
                .any(|table| changes.rows_changed[table.index()])
        {
            self.present |= frame.complete::<tables::RaidCountries>().is_ok_and(|rows| {
                rows.iter().any(|row| {
                    #[cfg(test)]
                    {
                        self.scan.work.row_visits += 1;
                    }
                    !row.eligible_types.is_empty()
                })
            }) || frame
                .complete::<tables::Raids>()
                .is_ok_and(|rows| !rows.is_empty());
        }
        let continuous = self.scan.last_frame.is_some_and(|(branch, hour)| {
            branch == frame.branch && hour.checked_add(1) == Some(frame.hour)
        });
        let (identity_changed, raid_changed) =
            self.scan.observe_changed(frame, content, Some(changes))?;
        self.events.through(frame.hour);
        self.tallies.advance(frame.hour)?;
        self.coverage.advance(frame.hour)?;
        self.reads.advance(frame.hour)?;
        let read = self.scan.result.read_hours.last() == Some(&frame.hour);
        self.scan.result.read_hours.clear();
        if !continuous || self.read_known != read {
            self.reads.observe(0, frame.hour, read.then_some(1), false);
        }
        self.read_known = read;
        if raid_changed {
            while self.ordinals.len() < self.scan.result.episodes.len() {
                self.ordinals.push(self.next_episode);
                self.next_episode = self
                    .next_episode
                    .checked_add(1)
                    .ok_or_else(|| dashboard_core::invalid("Raid episode count overflow"))?;
            }
            let open: BTreeSet<_> = self.scan.open.values().copied().collect();
            for (index, episode) in self.scan.result.episodes.iter_mut().enumerate() {
                #[cfg(test)]
                {
                    self.scan.work.history_visits += 1;
                }
                let keep = usize::from(open.contains(&index));
                let runs = episode
                    .runs
                    .drain(..episode.runs.len().saturating_sub(keep))
                    .collect();
                let preparation = episode
                    .preparation
                    .drain(..episode.preparation.len().saturating_sub(keep))
                    .collect();
                let chunk = EpisodeChunk {
                    ordinal: self.ordinals[index],
                    episode: RaidEpisode {
                        id: episode.id.clone(),
                        start: episode.start.clone(),
                        runs,
                        preparation,
                    },
                };
                if !chunk.episode.runs.is_empty() || !chunk.episode.preparation.is_empty() {
                    self.events.event(0, frame.hour, &chunk)?;
                    #[cfg(test)]
                    {
                        self.scan.work.emitted += 1;
                    }
                }
            }
            let mut remap = BTreeMap::new();
            let mut ordinals = Vec::new();
            self.scan.result.episodes = std::mem::take(&mut self.scan.result.episodes)
                .into_iter()
                .enumerate()
                .filter_map(|(index, episode)| {
                    #[cfg(test)]
                    {
                        self.scan.work.history_visits += 1;
                    }
                    if !open.contains(&index) {
                        return None;
                    }
                    remap.insert(index, ordinals.len());
                    ordinals.push(self.ordinals[index]);
                    Some(episode)
                })
                .collect();
            self.ordinals = ordinals;
            for index in self.scan.open.values_mut() {
                #[cfg(test)]
                {
                    self.scan.work.history_visits += 1;
                }
                *index = remap[index];
            }
        }
        if identity_changed {
            let mut next = BTreeSet::new();
            for tag in &self.scan.present_tags {
                #[cfg(test)]
                {
                    self.scan.work.history_visits += 1;
                }
                let id = u64::try_from(self.keys.len())
                    .ok()
                    .and_then(|n| n.checked_add(1))
                    .ok_or_else(|| dashboard_core::invalid("Raid country key count overflow"))?;
                let key = *self.keys.entry(tag.clone()).or_insert(id);
                next.insert(key);
                let track = self
                    .scan
                    .tracks
                    .get_mut(tag)
                    .expect("present country track");
                let segment = track.segments.last().expect("present segment");
                self.coverage
                    .observe(key, frame.hour, Some(1), segment.first_hour == frame.hour);
                if track.segments.len() > 1 {
                    track.segments.drain(..track.segments.len() - 1);
                }
            }
            for key in self.covered.difference(&next) {
                self.coverage.observe(*key, frame.hour, None, false);
            }
            self.covered = next;
        }
        let supplied = self.scan.raid_through == Some(frame.hour);
        let membership_changed = identity_changed
            || self.scan.parents_changed
            || !continuous
            || (supplied && self.supplied.is_empty());
        if membership_changed || (!supplied && !self.supplied.is_empty()) {
            let mut next = BTreeSet::new();
            if supplied {
                for tag in &self.scan.raid_tags {
                    #[cfg(test)]
                    {
                        self.scan.work.history_visits += 1;
                    }
                    next.insert(self.keys[tag]);
                }
            }
            for key in self.supplied.difference(&next) {
                self.tallies.observe(*key, frame.hour, None, false);
            }
            self.supplied = next;
        }
        if supplied && (membership_changed || !self.scan.changed_tallies.is_empty()) {
            let tags = if membership_changed {
                &self.scan.raid_tags
            } else {
                &self.scan.changed_tallies
            };
            for tag in tags {
                #[cfg(test)]
                {
                    self.scan.work.history_visits += 1;
                }
                let key = self.keys[tag];
                let holder = self.scan.tracks[tag]
                    .holder
                    .expect("supplying country holder");
                let (launched, succeeded) =
                    self.scan.tallies.get(&holder).copied().unwrap_or_default();
                let value = (i128::from(launched) << 32) | i128::from(succeeded);
                self.tallies.observe(key, frame.hour, Some(value), false);
                #[cfg(test)]
                {
                    self.scan.work.emitted += 1;
                }
            }
        }
        self.scan.changed_tallies.clear();
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self.events.bytes() + self.tallies.bytes() + self.coverage.bytes() + self.reads.bytes()
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.seal_presence();
        self.scan.seal_raids();
        self.events
            .flush(writer, &format!("raids-{branch}-events"), 0, cancelled)?;
        self.reads
            .flush(writer, &format!("raids-{branch}-reads"), 0, cancelled)?;
        self.tallies
            .flush(writer, &format!("raids-{branch}-tallies"), 0, cancelled)?;
        self.coverage
            .flush(writer, &format!("raids-{branch}-coverage"), 0, cancelled)?;
        series::write_catalogue(writer, "raids", branch, &self.keys, &())?;
        let bytes =
            dashboard_base::prepared::series::compress(serde_json::to_vec(self)?.as_slice(), 3)?;
        writer.write_at(&format!("raids-{branch}"), 0, bytes.as_slice())
    }
    pub fn read(
        store: &store::Store,
        key: &store::SourceKey,
        saved: Option<&store::SourceKey>,
        branch: u64,
    ) -> io::Result<Self> {
        let section = format!("raids-{branch}");
        let file = match saved {
            Some(previous) if previous != key => store.read_prefix(previous, key, &section, 0)?,
            _ => store.read(key, &section, 0)?,
        }
        .ok_or_else(|| dashboard_core::invalid("Prepared raid continuation is missing"))?;
        let mut state: Self = serde_json::from_reader(zstd::stream::Decoder::new(file)?)?;
        let catalogue: Option<series::Catalogue<()>> =
            series::read_saved_catalogue(store, key, saved, "raids", branch)?;
        state.keys = catalogue
            .ok_or_else(|| dashboard_core::invalid("Prepared raid catalogue is missing"))?
            .0;
        Ok(state)
    }
    fn scanned(
        mut self,
        store: &store::Store,
        key: &store::SourceKey,
        branch: u64,
        view: RaidsView,
        cancelled: &AtomicBool,
    ) -> io::Result<ScannedRaids> {
        self.scan.seal_presence();
        self.scan.seal_raids();
        self.scan.presence_through = None;
        self.scan.raid_through = None;
        let mut events = Reader::new(store, key, format!("raids-{branch}-events"), cancelled)?;
        let mut reads = Reader::new(store, key, format!("raids-{branch}-reads"), cancelled)?;
        for run in reads.runs(0, i32::MIN, i32::MAX)? {
            self.scan
                .result
                .read_hours
                .extend(run.first_hour..=run.last_hour);
        }
        if view == RaidsView::Log {
            let mut episodes: BTreeMap<u64, RaidEpisode> = BTreeMap::new();
            let mut cursor = 0;
            loop {
                let page = events.events(&[0], i32::MIN, i32::MAX, cursor, 1024)?;
                for row in page.rows {
                    check_cancel(cancelled)?;
                    let chunk: EpisodeChunk = serde_json::from_value(row.row)?;
                    let episode = episodes
                        .entry(chunk.ordinal)
                        .or_insert_with(|| RaidEpisode {
                            id: chunk.episode.id.clone(),
                            start: chunk.episode.start.clone(),
                            runs: Vec::new(),
                            preparation: Vec::new(),
                        });
                    episode.runs.extend(chunk.episode.runs);
                    episode.preparation.extend(chunk.episode.preparation);
                }
                let Some(next) = page.cursor else {
                    break;
                };
                cursor = next;
            }
            for (ordinal, suffix) in self
                .ordinals
                .into_iter()
                .zip(std::mem::take(&mut self.scan.result.episodes))
            {
                let episode = episodes.entry(ordinal).or_insert_with(|| RaidEpisode {
                    id: suffix.id.clone(),
                    start: suffix.start.clone(),
                    runs: Vec::new(),
                    preparation: Vec::new(),
                });
                episode.runs.extend(suffix.runs);
                episode.preparation.extend(suffix.preparation);
            }
            self.scan.result.episodes = episodes.into_values().collect();
        } else {
            self.scan.result.episodes.clear();
            let mut tallies =
                Reader::new(store, key, format!("raids-{branch}-tallies"), cancelled)?;
            let mut coverage =
                Reader::new(store, key, format!("raids-{branch}-coverage"), cancelled)?;
            for (tag, &id) in &self.keys {
                self.scan.result.series.insert(
                    tag.clone(),
                    tallies
                        .runs(id, i32::MIN, i32::MAX)?
                        .into_iter()
                        .map(|run| RaidTallyRun {
                            first_hour: run.first_hour,
                            last_hour: run.last_hour,
                            read_hour: run.first_hour,
                            launched: (run.value >> 32) as u32,
                            succeeded: (run.value & i128::from(u32::MAX)) as u32,
                        })
                        .collect(),
                );
                self.scan.result.segments.insert(
                    tag.clone(),
                    coverage
                        .runs(id, i32::MIN, i32::MAX)?
                        .into_iter()
                        .map(|run| HourSpan {
                            first_hour: run.first_hour,
                            last_hour: run.last_hour,
                        })
                        .collect(),
                );
            }
        }
        // Tracks contain only observer continuation; reconstructed histories are above.
        self.scan.tracks.clear();
        Ok(self.scan.finish())
    }
}
pub fn query(
    source: &Source,
    params: serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let params: RaidsParams = serde_json::from_value(params)?;
    Raids::validate(&params).map_err(dashboard_core::invalid)?;
    let branch = params.branch_id.parse::<u64>().expect("validated branch");
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|row| row.id == branch)
    {
        return Err(dashboard_core::invalid("Branch is not in this recording"));
    }
    if source.block_windows(branch).is_empty() {
        return Err(dashboard_core::invalid("Empty raid branch"));
    }
    let mut prefix = dashboard_base::prepared::prefix::read_or_empty(
        source,
        context.content.as_ref(),
        if params.view == RaidsView::Tally {
            "raids-tally"
        } else {
            Raids::KEY
        },
        branch,
        None,
        cancelled,
    )?;
    let accepted =
        (prefix.coverage.first_hour <= prefix.coverage.last_hour).then(|| prefix.coverage.clone());
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let state = if accepted.is_some() {
        Prepared::read(&store, &key, None, branch)?
    } else {
        Prepared::default()
    };
    let (content, warnings) = prepared_content(source, context)?;
    let available = if state.present || content.as_ref().is_some_and(|c| !c.raids.types.is_empty())
    {
        Some(true)
    } else {
        content
            .as_ref()
            .filter(|c| c.raids.uncertain.is_empty())
            .map(|_| false)
    };
    let histories = if params.view == RaidsView::Tally {
        let histories = dashboard_base::prepared::query::progressive::histories(
            &store,
            &key,
            branch,
            accepted.as_ref(),
            &Default::default(),
            &Default::default(),
            source.replay_ref(),
            &params.tags,
            cancelled,
        )?;
        Some(histories)
    } else {
        None
    };
    let mut scanned = if params.view == RaidsView::Catalogue {
        None
    } else {
        Some(state.scanned(&store, &key, branch, params.view, cancelled)?)
    };
    let mut exact = None;
    if params.view == RaidsView::Tally
        && !prefix.coverage.complete
        && let Some(scanned) = &mut scanned
    {
        let last = prefix.coverage.last_hour;
        scanned.read_hours.retain(|hour| *hour <= last);
        for runs in scanned.series.values_mut() {
            runs.retain(|run| run.first_hour <= last);
            for run in runs {
                run.last_hour = run.last_hour.min(last);
            }
        }
        for spans in scanned.segments.values_mut() {
            spans.retain(|span| span.first_hour <= last);
            for span in spans {
                span.last_hour = span.last_hour.min(last);
            }
        }
    }
    if params.view == RaidsView::Log && !prefix.coverage.complete {
        if let Some((newest, span)) =
            newest::scan(source, branch, content.as_deref(), &key, cancelled)?
        {
            let base = scanned.get_or_insert_with(ScannedRaids::default);
            let mut episodes: BTreeMap<_, _> = std::mem::take(&mut base.episodes)
                .into_iter()
                .map(|episode| (episode.id.clone(), episode))
                .collect();
            episodes.extend(
                newest
                    .episodes
                    .into_iter()
                    .map(|episode| (episode.id.clone(), episode)),
            );
            base.episodes = episodes.into_values().collect();
            base.read_hours.extend(newest.read_hours);
            base.read_hours.sort_unstable();
            base.read_hours.dedup();
            base.types.extend(newest.types);
            base.recording_present |= newest.recording_present;
            base.warnings.extend(newest.warnings);
            exact = Some(span.first_hour);
            prefix.sampled(&[span]);
        }
    }
    check_cancel(cancelled)?;
    let mut data = serde_json::to_value(reply(
        &params,
        content.as_deref(),
        available,
        warnings,
        scanned.as_ref(),
        histories.as_ref(),
        cancelled,
    )?)?;
    prefix.mark_coverage(&mut data, "coverage")?;
    if let Some(exact) = exact {
        data["warningsComplete"] = serde_json::Value::Bool(false);
        data["exact"] = serde_json::Value::from(exact);
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{frame, raid};
    use super::*;
    use dashboard_base::prepared::series::tests::Fixture;
    use dashboard_core::test_fixtures::scale_frame;
    use replay::{
        frame::TableView,
        rows::{RaidCountryRow, TableRows},
    };

    #[test]
    fn raids_shared_driver_equal_reads_materialise_preparation_at_the_exact_boundary() {
        use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let measured = observer_tests::scaled_work(8, |hours| {
            let fixture = Fixture::new();
            let flag = AtomicBool::new(false);
            let mut rising = frame(100, vec![raid(1, 2, 0)]);
            let mut first = rising.clone();
            first.hour = 101;
            if let TableView::Complete(TableRows::Raids(rows)) =
                &mut *rising.tables[TableId::Raids.index()]
            {
                rows[0].preparation_hours = 4;
            }
            if let TableView::Complete(TableRows::Raids(rows)) =
                &mut *first.tables[TableId::Raids.index()]
            {
                rows[0].preparation_hours = 5;
            }
            let replay = dashboard_core::test_fixtures::replay(vec![rising.clone(), first.clone()]);
            let mut state = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            state.observe_changed(
                &rising,
                replay.format,
                None,
                &tracker.observe(&rising, replay.format),
            )?;
            state.observe_changed(
                &first,
                replay.format,
                None,
                &tracker.observe(&first, replay.format),
            )?;
            let delta = observer_tests::unchanged_frames(
                state.scan.work,
                &first,
                hours as i32,
                replay.format,
                |read, changes| {
                    state.observe_changed(read, replay.format, None, changes)?;
                    Ok(state.scan.work)
                },
            )?;
            assert_eq!(delta, observer_tests::Work::default());
            assert_eq!(state.scan.result.episodes[0].runs[0].last_hour, first.hour);
            let mut writer = fixture.writer();
            state.flush(&mut writer, 1, &flag)?;
            writer.commit()?;
            let mut frames = vec![rising];
            frames.extend((0..=hours).map(|offset| {
                let mut frame = first.clone();
                frame.hour += offset as i32;
                frame
            }));
            parity(&fixture, &frames);
            let episode = &state.scan.result.episodes[0];
            assert_eq!(
                episode.preparation,
                vec![
                    RaidPreparationRun(100, 101, 4, 1),
                    RaidPreparationRun(102, 101 + hours as i32, 5, 0)
                ]
            );
            Ok(delta)
        })
        .unwrap();
        observer_tests::assert_flat(measured);
    }

    #[test]
    fn raids_shared_driver_one_raid_change_does_not_visit_closed_episode_history() {
        use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let measured = observer_tests::scaled_work(20, |history| {
            let mut state = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            let mut input = frame(100, vec![raid(1, 2, 0)]);
            let replay = dashboard_core::test_fixtures::replay(vec![input.clone()]);
            for offset in 0..history {
                input.hour = 100 + offset as i32;
                if let TableView::Complete(TableRows::Raids(rows)) =
                    &mut *input.tables[TableId::Raids.index()]
                {
                    rows[0].id = offset as u32 + 1;
                }
                state.observe_changed(
                    &input,
                    replay.format,
                    None,
                    &tracker.observe(&input, replay.format),
                )?;
            }
            assert_eq!(state.scan.result.episodes.len(), 1);
            let before = state.scan.work;
            let delta = observer_tests::unchanged_frames(
                before,
                &input,
                10,
                replay.format,
                |frame, changes| {
                    state.observe_changed(frame, replay.format, None, changes)?;
                    Ok(state.scan.work)
                },
            )?;
            assert_eq!(delta, observer_tests::Work::default());
            let mut previous = input.clone();
            previous.hour += 10;
            tracker.observe(&previous, replay.format);
            input.hour += 11;
            if let TableView::Complete(TableRows::Raids(rows)) =
                &mut *input.tables[TableId::Raids.index()]
            {
                rows[0].phase = native::PHASE_IN_PROGRESS;
            }
            state.observe_changed(
                &input,
                replay.format,
                None,
                &tracker.observe(&input, replay.format),
            )?;
            let changed = state.scan.work;
            let work = observer_tests::Work {
                row_visits: changed.row_visits - before.row_visits,
                history_visits: changed.history_visits - before.history_visits,
                emitted: changed.emitted - before.emitted,
                ..Default::default()
            };
            assert!(work.row_visits < 100 && work.history_visits < 20);
            Ok(work)
        })
        .unwrap();
        observer_tests::assert_flat(measured);
    }

    fn parity(fixture: &Fixture, frames: &[DecodedFrame]) {
        let flag = AtomicBool::new(false);
        let replay = dashboard_core::test_fixtures::replay(frames.to_vec());
        let oracle = scan(&replay, 1, None, &flag).unwrap();
        for view in [RaidsView::Log, RaidsView::Tally] {
            for metric in [
                RaidMetric::Launched,
                RaidMetric::Succeeded,
                RaidMetric::Both,
            ] {
                let params = RaidsParams {
                    branch_id: "1".into(),
                    view,
                    tags: vec![],
                    top: (view == RaidsView::Tally).then_some(RaidTop {
                        count: 5000,
                        metric,
                    }),
                };
                let actual = Prepared::read(&fixture.store, &fixture.key, None, 1)
                    .unwrap()
                    .scanned(&fixture.store, &fixture.key, 1, view, &flag)
                    .unwrap();
                let expected = reply(
                    &params,
                    None,
                    Some(true),
                    vec![],
                    Some(&oracle),
                    None,
                    &flag,
                )
                .unwrap();
                let actual = reply(
                    &params,
                    None,
                    Some(true),
                    vec![],
                    Some(&actual),
                    None,
                    &flag,
                )
                .unwrap();
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
            }
        }
    }
    #[test]
    fn prepared_raids_two_publications_resume_append_preserve_episodes_and_witnesses() {
        let mut fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut frames = vec![
            frame(100, vec![raid(1, 4, 0), raid(2, 2, 0)]),
            frame(101, vec![raid(2, 2, 0)]),
            frame(102, vec![raid(1, 5, 3)]),
            frame(104, vec![]),
            frame(105, vec![raid(1, 5, 3), raid(3, 4, 0)]),
        ];
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frames[4].tables[TableId::Countries.index()]
        {
            rows[0].tag = b"renamed holder".to_vec();
        }
        let mut state = Prepared::default();
        for chunk in [&frames[..1], &frames[1..3]] {
            let mut writer = fixture.writer();
            for frame in chunk {
                state
                    .observe_with_content(frame, replay::format::FormatVersion::default())
                    .unwrap();
            }
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            state = Prepared::read(&fixture.store, &fixture.key, None, 1).unwrap();
        }
        parity(&fixture, &frames[..3]);
        let previous = fixture.key.clone();
        let mut writer = fixture.append_writer();
        state = Prepared::read(&fixture.store, &fixture.key, Some(&previous), 1).unwrap();
        for frame in &frames[3..] {
            state
                .observe_with_content(frame, replay::format::FormatVersion::default())
                .unwrap();
        }
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        parity(&fixture, &frames);
        assert_eq!(state.scan.counted_launch.len(), 2);
        assert_eq!(state.scan.counted_success.len(), 1);
    }

    #[test]
    fn raids_deferred_endpoints_survive_two_commits_resume_append_and_a_gap() {
        use dashboard_base::prepared::frame_changes::FrameChangeTracker;
        let mut fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let first = frame(100, vec![raid(1, 2, 0)]);
        let mut frames: Vec<_> = (100..=129)
            .map(|hour| {
                let mut frame = first.clone();
                frame.hour = hour;
                frame
            })
            .collect();
        frames[29].hour = 131;
        let replay = dashboard_core::test_fixtures::replay(frames.clone());
        let mut state = Prepared::default();
        for chunk in [&frames[..10], &frames[10..20]] {
            let mut tracker = FrameChangeTracker::new();
            let mut writer = fixture.writer();
            for frame in chunk {
                state
                    .observe_changed(
                        frame,
                        replay.format,
                        None,
                        &tracker.observe(frame, replay.format),
                    )
                    .unwrap();
            }
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            state = Prepared::read(&fixture.store, &fixture.key, None, 1).unwrap();
        }
        parity(&fixture, &frames[..20]);
        let previous = fixture.key.clone();
        let mut writer = fixture.append_writer();
        state = Prepared::read(&fixture.store, &fixture.key, Some(&previous), 1).unwrap();
        let mut tracker = FrameChangeTracker::new();
        for frame in &frames[20..] {
            state
                .observe_changed(
                    frame,
                    replay.format,
                    None,
                    &tracker.observe(frame, replay.format),
                )
                .unwrap();
        }
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        parity(&fixture, &frames);
    }
    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn prepared_raids_keep_5000_country_tallies_and_identity_gaps() {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut first = scale_frame(1, 100, 5000);
        *first.tables[TableId::RaidCountries.index()] =
            TableView::Complete(TableRows::RaidCountries(
                (0..5000)
                    .map(|slot| RaidCountryRow {
                        canonical_slot: slot,
                        country: (100000 + slot) as i32,
                        eligible_types: vec![50],
                        ..Default::default()
                    })
                    .collect(),
            ));
        *first.tables[TableId::Raids.index()] = TableView::Complete(TableRows::Raids(vec![]));
        let mut last = first.clone();
        last.hour = 102;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *last.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"alias".to_vec();
        }
        let frames = vec![first, last];
        let mut state = Prepared::default();
        for frame in &frames {
            let mut writer = fixture.writer();
            state
                .observe_with_content(frame, replay::format::FormatVersion::default())
                .unwrap();
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            state = Prepared::read(&fixture.store, &fixture.key, None, 1).unwrap();
        }
        assert_eq!(state.keys.len(), 5001);
        parity(&fixture, &frames);
        let scanned = state
            .scanned(&fixture.store, &fixture.key, 1, RaidsView::Tally, &flag)
            .unwrap();
        assert_eq!(scanned.series.len(), 5001);
    }
    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn prepared_raids_continuation_retains_open_suffixes_and_no_hourly_history() {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut state = Prepared::default();
        let mut sizes = Vec::new();
        for end in [100, 10000] {
            let start = state
                .scan
                .tracks
                .get("GER")
                .and_then(|track| track.segments.last())
                .map_or(1, |span| span.last_hour + 1);
            let mut writer = fixture.writer();
            for hour in start..=end {
                let mut observed = frame(hour, vec![raid(1, 2, 0)]);
                if let TableView::Complete(TableRows::Raids(rows)) =
                    &mut *observed.tables[TableId::Raids.index()]
                {
                    rows[0].preparation_hours = hour;
                }
                state
                    .observe_with_content(&observed, replay::format::FormatVersion::default())
                    .unwrap();
            }
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            sizes.push(serde_json::to_vec(&state).unwrap().len());
            assert_eq!(state.scan.result.episodes.len(), 1);
            assert_eq!(state.scan.result.episodes[0].runs.len(), 1);
            assert_eq!(state.scan.result.episodes[0].preparation.len(), 1);
            assert!(state.scan.result.read_hours.is_empty());
            assert!(
                state
                    .scan
                    .tracks
                    .values()
                    .all(|track| track.runs.is_empty() && track.segments.len() == 1)
            );
            assert_eq!(state.scan.counted_launch.len(), 0);
        }
        eprintln!("raid continuation bytes at 100 and 10000 unchanged hours: {sizes:?}");
        assert!(sizes[1] < sizes[0] + 256);
    }
}
