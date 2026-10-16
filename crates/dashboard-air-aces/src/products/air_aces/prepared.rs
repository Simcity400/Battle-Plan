//! Block roster replies, sparse naming observations and compact portrait selections.
use super::*;
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use dashboard_text::metadata::check_cancel;

#[derive(Serialize, Deserialize)]
struct PortraitRun {
    first: i32,
    last: i32,
    operands: PortraitOperands,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    series: Accumulator,
    #[serde(skip)]
    keys: BTreeMap<String, u64>,
    portraits: BTreeMap<u64, Vec<PortraitRun>>,
    portrait_through: Option<i32>,
    portrait_active: BTreeSet<u64>,
    read_active: bool,
    /// The content refusals this family's own reads produced, which its
    /// published windows report.
    #[serde(default)]
    refusals: catalogue::OwnRefusals,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}
impl Prepared {
    /// Runs `run` as this family's own consumer of the shared content
    /// ([`catalogue::own_refusals`]).
    fn own<T>(&mut self, run: impl FnOnce(&mut Self) -> T) -> T {
        let mut own = std::mem::take(&mut self.refusals);
        let value = catalogue::own_refusals(&mut own, || run(self));
        self.refusals = own;
        value
    }
    #[cfg(test)]
    pub fn observe(&mut self, frame: &DecodedFrame, replay: &Replay) -> io::Result<()> {
        let changes = dashboard_base::prepared::frame_changes::FrameChangeTracker::new()
            .observe(frame, replay.format);
        self.observe_changed(frame, replay, &changes)
    }
    pub fn observe_changed(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.own(|state| state.observe_own_changes(frame, replay, changes))
    }
    fn observe_own_changes(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.scan.format = replay.format;
        let continuous = self
            .scan
            .last_frame
            .is_some_and(|hour| hour.checked_add(1) == Some(frame.hour));
        let versions = self
            .scan
            .variants
            .observe_changed(replay.format, frame, changes);
        let changed = self.scan.observe_roster(frame, Some(changes));
        self.series.advance(frame.hour)?;
        let read = self.scan.reads.last() == Some(&frame.hour);
        if !continuous || self.read_active != read {
            self.series.observe(0, frame.hour, read.then_some(1), false);
        }
        self.read_active = read;
        if self.scan.reads.len() > 1 {
            self.scan.reads.drain(..self.scan.reads.len() - 1);
        }
        for (id, version) in versions {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
                self.work.emitted += 1;
            }
            let next = u64::try_from(self.keys.len())
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| dashboard_core::invalid("Ace variant key count overflow"))?;
            let key = *self.keys.entry(id.to_string()).or_insert(next);
            self.series.event(key, frame.hour, &version)?;
            let versions = self
                .scan
                .variants
                .versions
                .get_mut(&id)
                .expect("changed variant");
            if versions.len() > 1 {
                versions.drain(..versions.len() - 1);
            }
        }
        if changed || !continuous {
            self.seal_portraits();
            self.portrait_active.clear();
            if self.scan.known {
                for &index in &self.scan.active {
                    #[cfg(test)]
                    {
                        self.work.history_visits += 1;
                    }
                    let ace = &self.scan.aces[index];
                    let operands =
                        PortraitOperands::from(&ace.runs.last().expect("active ace run").facts);
                    let runs = self.portraits.entry(ace.id).or_default();
                    if let Some(last) = runs.last_mut().filter(|run| {
                        run.last.checked_add(1) == Some(frame.hour) && run.operands == operands
                    }) {
                        last.last = frame.hour;
                    } else {
                        runs.push(PortraitRun {
                            first: frame.hour,
                            last: frame.hour,
                            operands,
                        });
                        #[cfg(test)]
                        {
                            self.work.emitted += 1;
                        }
                    }
                    self.portrait_active.insert(ace.id);
                }
            }
        }
        self.portrait_through = self.scan.known.then_some(frame.hour);
        Ok(())
    }
    fn seal_portraits(&mut self) {
        if let Some(hour) = self.portrait_through {
            for id in &self.portrait_active {
                #[cfg(test)]
                {
                    self.work.history_visits += 1;
                }
                self.portraits
                    .get_mut(id)
                    .and_then(|runs| runs.last_mut())
                    .expect("active portrait")
                    .last = hour;
            }
        }
    }
    pub fn bytes(&self) -> usize {
        self.series.bytes()
    }

    pub fn finish_block(
        &mut self,
        writer: &mut store::Writer,
        source: &Source,
        replay: &Replay,
        block: usize,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.own(|state| state.finish_own_block(writer, source, replay, block, context, cancelled))
    }
    fn finish_own_block(
        &mut self,
        writer: &mut store::Writer,
        source: &Source,
        replay: &Replay,
        block: usize,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.seal_portraits();
        let index = &replay.index[block];
        let window = replay.index[..block]
            .iter()
            .filter(|row| row.branch == index.branch)
            .count();
        self.scan.windows = replay
            .index
            .iter()
            .filter(|row| row.branch == index.branch)
            .map(|row| HourSpan {
                first_hour: row.first_hour,
                last_hour: row.last_hour,
            })
            .collect();
        if let Some(hour) = self.scan.last_frame {
            self.scan.close(hour);
        }
        let cat = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options));
        let params = AirAcesParams {
            branch_id: index.branch.to_string(),
            view: AirAcesView::Window,
            window: Some(window),
        };
        let mut reply = AirAcesReply::Window(window_reply(
            &self.scan,
            &params,
            cat.as_deref().ok(),
            cancelled,
        )?);
        ace_warnings(&mut reply, cat.as_deref());
        let AirAcesReply::Window(reply) = bounded(reply, "window")? else {
            unreachable!()
        };
        series::write_reply(
            writer,
            &format!("air-aces-{}-windows", index.branch),
            window as u64,
            &reply,
        )?;
        // Completed runs live only in the window files. All ace identities keep their bounds.
        let active: HashSet<_> = self.scan.active.iter().copied().collect();
        for (index, ace) in self.scan.aces.iter_mut().enumerate() {
            if active.contains(&index) {
                if ace.runs.len() > 1 {
                    ace.runs.drain(..ace.runs.len() - 1);
                }
            } else {
                ace.runs.clear();
            }
        }
        Ok(())
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.seal_portraits();
        if let Some(hour) = self.scan.last_frame {
            self.scan.close(hour);
        }
        self.series
            .flush(writer, &format!("air-aces-{branch}-series"), 0, cancelled)?;
        series::write_catalogue(writer, "air-aces", branch, &self.keys, &())?;
        let bytes =
            dashboard_base::prepared::series::compress(serde_json::to_vec(self)?.as_slice(), 3)?;
        writer.write_at(&format!("air-aces-{branch}"), 0, bytes.as_slice())
    }
    fn read(
        store: &store::Store,
        key: &store::SourceKey,
        saved: Option<&store::SourceKey>,
        branch: u64,
    ) -> io::Result<Self> {
        let section = format!("air-aces-{branch}");
        let file = match saved {
            Some(previous) if previous != key => store.read_prefix(previous, key, &section, 0)?,
            _ => store.read(key, &section, 0)?,
        }
        .ok_or_else(|| dashboard_core::invalid("Prepared ace continuation is missing"))?;
        serde_json::from_reader(zstd::stream::Decoder::new(file)?).map_err(Into::into)
    }
    pub fn resume(
        store: &store::Store,
        key: &store::SourceKey,
        saved: Option<&store::SourceKey>,
        replay: &Replay,
        branch: u64,
    ) -> io::Result<Self> {
        let mut state = Self::read(store, key, saved, branch)?;
        let catalogue: Option<series::Catalogue<()>> =
            series::read_saved_catalogue(store, key, saved, "air-aces", branch)?;
        state.keys = catalogue
            .ok_or_else(|| dashboard_core::invalid("Prepared ace catalogue is missing"))?
            .0;
        if state.scan.active.is_empty() {
            return Ok(state);
        }
        let file = match saved {
            Some(previous) if previous != key => {
                store.read_prefix(previous, key, "map.resume", 0)?
            }
            _ => store.read(key, "map.resume", 0)?,
        }
        .ok_or_else(|| dashboard_core::invalid("Ace decoder continuation is missing"))?;
        // Shared pipeline checkpoints contain every table. The smaller selection
        // applies only to the interleaved-branch fallback seek below.
        let baseline = dashboard_base::prepared_map::decode_checkpoint(file, &TableId::ALL)?.frame;
        let baseline = if baseline.branch == branch && Some(baseline.hour) == state.scan.last_frame
        {
            baseline
        } else {
            // Interleaved branches share one decoder resume page. Restore one hour,
            // during preparation only, when that page belongs to another branch.
            let hour = state
                .scan
                .last_frame
                .ok_or_else(|| dashboard_core::invalid("Ace continuation hour is missing"))?;
            let replay::container::SeekResult::Frame(frame) =
                replay.seek_selected(branch, hour, Some(&[TableId::Campaign, TableId::AirAces]))?
            else {
                return Err(dashboard_core::invalid(
                    "Ace continuation frame is unavailable",
                ));
            };
            *frame
        };
        state.restore_frame(&baseline)?;
        Ok(state)
    }
    fn restore_frame(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let rows = frame.complete::<tables::AirAces>()?;
        let indices: HashMap<_, _> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id, index))
            .collect();
        for &index in &self.scan.active {
            let ace = &mut self.scan.aces[index];
            let position = *indices
                .get(&ace.id)
                .ok_or_else(|| dashboard_core::invalid("Ace continuation row is missing"))?;
            ace.runs
                .last_mut()
                .ok_or_else(|| dashboard_core::invalid("Ace continuation run is missing"))?
                .facts
                .row = RecordedAce::Initial {
                table: frame.tables[TableId::AirAces.index()].clone(),
                index: position,
            };
        }
        Ok(())
    }
    fn manifest(
        &mut self,
        source: &Source,
        store: &store::Store,
        key: &store::SourceKey,
        params: &AirAcesParams,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<AirAcesReply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let replay = source.replay_ref();
        let declaration = replay
            .branches
            .iter()
            .find(|row| row.id == branch)
            .ok_or_else(|| dashboard_core::invalid("Branch not in this recording"))?;
        (self.scan.first, self.scan.last) = branch_bounds(replay, declaration);
        self.scan.windows = replay
            .index
            .iter()
            .filter(|row| row.branch == branch)
            .map(|row| HourSpan {
                first_hour: row.first_hour,
                last_hour: row.last_hour,
            })
            .collect();
        self.scan.reads.clear();
        self.scan.variants.versions.clear();
        let (keys, ()) = series::read_catalogue::<()>(store, key, "air-aces", branch)?
            .ok_or_else(|| dashboard_core::invalid("Prepared ace catalogue is missing"))?;
        let variants: BTreeMap<_, _> = keys
            .into_iter()
            .map(|(id, key)| {
                id.parse::<u64>()
                    .map(|id| (key, id))
                    .map_err(|_| dashboard_core::invalid("Invalid ace variant key"))
            })
            .collect::<io::Result<_>>()?;
        let wanted: Vec<_> = variants.keys().copied().collect();
        let mut reader = Reader::new(store, key, format!("air-aces-{branch}-series"), cancelled)?;
        for run in reader.runs(0, i32::MIN, i32::MAX)? {
            self.scan.reads.extend(run.first_hour..=run.last_hour);
        }
        let mut cursor = 0;
        loop {
            let page = reader.events(&wanted, i32::MIN, i32::MAX, cursor, 1024)?;
            for row in page.rows {
                {
                    let id = variants[&row.key];
                    self.scan
                        .variants
                        .versions
                        .entry(id)
                        .or_default()
                        .push(serde_json::from_value(row.row)?);
                }
            }
            let Some(next) = page.cursor else {
                break;
            };
            cursor = next;
        }
        let cat = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options));
        let mut reply = AirAcesReply::Manifest(manifest(
            &self.scan,
            params,
            cat.as_deref().ok(),
            cancelled,
        )?);
        ace_warnings(&mut reply, cat.as_deref());
        bounded(reply, "manifest")
    }
    fn portrait_at(&self, id: u64, hour: i32) -> Option<PortraitOperands> {
        let runs = self.portraits.get(&id)?;
        let index = runs
            .partition_point(|run| run.first <= hour)
            .checked_sub(1)?;
        let run = &runs[index];
        let last = if index + 1 == runs.len() && self.portrait_active.contains(&id) {
            self.portrait_through.unwrap_or(run.last)
        } else {
            run.last
        };
        (hour <= last).then(|| run.operands.clone())
    }
}

pub fn query(
    source: &Source,
    product: &str,
    params: serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let branch = params
        .get("branchId")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| dashboard_core::invalid("Invalid branch"))?;
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|row| row.id == branch)
    {
        return Err(dashboard_core::invalid("Branch not in this recording"));
    }
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    if product == AirAcePortraits::KEY {
        let params: AirAcePortraitParams = serde_json::from_value(params)?;
        AirAcePortraits::validate(&params).map_err(dashboard_core::invalid)?;
        let branch = params.branch_id.parse().expect("validated branch");
        let Some(coverage) = dashboard_base::prepared::coverage::coverage(
            source,
            context.content.as_ref(),
            branch,
            cancelled,
        )?
        else {
            return Ok(serde_json::Value::Null);
        };
        if !coverage.complete && params.hour > coverage.last_hour {
            return Ok(serde_json::Value::Null);
        }
        let state = Prepared::read(&store, &key, None, branch)?;
        return serde_json::to_value(portrait_reply(
            source,
            &params,
            context,
            cancelled,
            |id, hour| state.portrait_at(id, hour),
        )?)
        .map_err(Into::into);
    }
    let params: AirAcesParams = serde_json::from_value(params)?;
    AirAces::validate(&params).map_err(dashboard_core::invalid)?;
    let branch = params.branch_id.parse().expect("validated branch");
    let Some(coverage) = dashboard_base::prepared::coverage::coverage(
        source,
        context.content.as_ref(),
        branch,
        cancelled,
    )?
    else {
        return Ok(serde_json::Value::Null);
    };
    if matches!(params.view, AirAcesView::Window) {
        let window = params.window.expect("validated window") as u64;
        if window
            >= source
                .replay_ref()
                .index
                .iter()
                .filter(|row| row.branch == branch)
                .count() as u64
        {
            return Err(dashboard_core::invalid("Ace window not in this recording"));
        }
        let reply = series::read_reply::<AirAcesWindowReply>(
            &store,
            &key,
            &format!("air-aces-{branch}-windows"),
            window,
        )?;
        return reply
            .map(serde_json::to_value)
            .transpose()
            .map(|value| value.unwrap_or(serde_json::Value::Null))
            .map_err(Into::into);
    }
    if !coverage.complete {
        return Ok(serde_json::Value::Null);
    }
    let mut state = Prepared::read(&store, &key, None, branch)?;
    if matches!(params.view, AirAcesView::Catalogue) {
        return Ok(
            serde_json::json!({"available": !state.scan.observed.is_empty(),
            "unavailable": if state.scan.observed.is_empty() { Some(if state.scan.unread {
                "AirAces availability could not be checked"
            } else { "AirAces was unavailable, incomplete or never read in this recording." }) } else { None }}),
        );
    }
    check_cancel(cancelled)?;
    serde_json::to_value(state.manifest(source, &store, &key, &params, context, cancelled)?)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::prepared::series::tests::Fixture;
    use dashboard_core::test_fixtures::{CAMPAIGN, continuation_records, records, scale_frame};
    use replay::rows::{AceDeath, TableRows};
    use std::num::NonZeroU32;

    fn work(state: &Prepared) -> dashboard_base::prepared::observer_tests::Work {
        let scan = state.scan.work;
        let variants = state.scan.variants.work;
        dashboard_base::prepared::observer_tests::Work {
            row_visits: scan.row_visits + variants.row_visits + state.work.row_visits,
            history_visits: scan.history_visits
                + variants.history_visits
                + state.work.history_visits,
            emitted: scan.emitted + variants.emitted + state.work.emitted,
            ..Default::default()
        }
    }

    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "5,000-country observer work and reply parity"
    )]
    fn aces_shared_driver_equal_reads_defer_roster_and_portrait_dates() {
        use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let measured = observer_tests::scaled_work(8, |hours| {
            let fixture = Fixture::new();
            let flag = AtomicBool::new(false);
            let mut first = scale_frame(1, 10, 5000);
            first.metrics.table_ns[TableId::AirAces.index()] = 1;
            first.tables[TableId::AirAces.index()] = TableView::Complete(TableRows::AirAces(
                (0..5000)
                    .map(|id| AirAceRow {
                        country: 100_000 + id,
                        ..super::super::tests::ace(u64::from(id) + 1, true)
                    })
                    .collect(),
            ))
            .into();
            let frames: Vec<_> = (0..=hours)
                .map(|offset| {
                    let mut frame = first.clone();
                    frame.hour += offset as i32;
                    frame
                })
                .collect();
            let source = source(Replay::from_records(
                CAMPAIGN.into(),
                &[records(1, frames)],
            )?);
            let mut state = Prepared::default();
            state.observe_changed(
                &first,
                source.replay_ref(),
                &FrameChangeTracker::new().observe(&first, source.replay_ref().format),
            )?;
            let delta = observer_tests::unchanged_frames(
                work(&state),
                &first,
                hours as i32,
                source.replay_ref().format,
                |frame, changes| {
                    state.observe_changed(frame, source.replay_ref(), changes)?;
                    Ok(work(&state))
                },
            )?;
            assert_eq!(delta, observer_tests::Work::default());
            assert_eq!(state.scan.aces[0].last_alive, Some(first.hour));
            assert!(state.portrait_at(1, first.hour + hours as i32).is_some());
            let mut writer = fixture.writer();
            state.finish_block(
                &mut writer,
                &source,
                source.replay_ref(),
                0,
                &LoadContext { content: None },
                &flag,
            )?;
            state.flush(&mut writer, 1, &flag)?;
            writer.commit()?;
            assert_parity(&fixture, &source);
            Ok(delta)
        })
        .unwrap();
        observer_tests::assert_flat(measured);
    }

    #[test]
    fn aces_shared_driver_unchanged_work_does_not_visit_closed_rosters() {
        use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let measured = observer_tests::scaled_work(20, |history| {
            let mut frame = super::super::tests::frame(10, vec![super::super::tests::ace(1, true)]);
            let replay = dashboard_core::test_fixtures::replay(vec![frame.clone()]);
            let mut state = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            for offset in 0..history {
                frame.hour = 10 + offset as i32;
                if let TableView::Complete(TableRows::AirAces(rows)) =
                    &mut *frame.tables[TableId::AirAces.index()]
                {
                    rows[0].id = offset as u64 + 1;
                }
                state.observe_changed(&frame, &replay, &tracker.observe(&frame, replay.format))?;
            }
            assert_eq!(state.scan.aces.len(), history);
            let delta = observer_tests::unchanged_frames(
                work(&state),
                &frame,
                10,
                replay.format,
                |frame, changes| {
                    state.observe_changed(frame, &replay, changes)?;
                    Ok(work(&state))
                },
            )?;
            let before = work(&state);
            let mut previous = frame.clone();
            previous.hour += 10;
            frame.hour += 11;
            if let TableView::Complete(TableRows::AirAces(rows)) =
                &mut *frame.tables[TableId::AirAces.index()]
            {
                rows[0].name = b"changed current ace".to_vec();
            }
            // Feed the previous equal frame so this is an adjacent one-row change.
            let mut tracker = FrameChangeTracker::new();
            tracker.observe(&previous, replay.format);
            state.observe_changed(&frame, &replay, &tracker.observe(&frame, replay.format))?;
            let changed = work(&state);
            assert!(changed.row_visits - before.row_visits < 100);
            assert!(changed.history_visits - before.history_visits < 10);
            Ok(delta)
        })
        .unwrap();
        assert_eq!(measured, [observer_tests::Work::default(); 2]);
        observer_tests::assert_flat(measured);
    }

    fn source(replay: Replay) -> Source {
        Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "prepared-aces".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap()
    }
    fn publish(
        state: &mut Prepared,
        writer: &mut store::Writer,
        map: &mut dashboard_base::prepared_map::Builder,
        source: &Source,
        block: usize,
    ) {
        let flag = AtomicBool::new(false);
        let updates = source
            .replay_ref()
            .read_block_selected(&source.replay_ref().index[block], &TableId::ALL)
            .unwrap();
        let frame_count = updates.len();
        let mut decoder = replay::frame::FrameDecoder::for_tables(&TableId::ALL);
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        for (next, update) in updates.into_iter().enumerate() {
            let frame = decoder.advance(&update).unwrap();
            state
                .observe_changed(
                    frame,
                    source.replay_ref(),
                    &tracker.observe(frame, source.replay_ref().format),
                )
                .unwrap();
            map.accept(
                writer,
                &update,
                frame,
                dashboard_base::prepared_map::Position {
                    block,
                    next_frame: next + 1,
                },
            )
            .unwrap();
        }
        state
            .finish_block(
                writer,
                source,
                source.replay_ref(),
                block,
                &LoadContext { content: None },
                &flag,
            )
            .unwrap();
        state.flush(writer, 1, &flag).unwrap();
        map.save_resume(
            writer,
            decoder.current().unwrap(),
            dashboard_base::prepared_map::Position {
                block,
                next_frame: frame_count,
            },
            &TableId::ALL,
        )
        .unwrap();
        assert!(state.scan.aces.iter().all(|ace| ace.runs.len() <= 1));
    }
    fn assert_parity(fixture: &Fixture, source: &Source) {
        let flag = AtomicBool::new(false);
        let context = LoadContext { content: None };
        let params = AirAcesParams {
            branch_id: "1".into(),
            view: AirAcesView::Manifest,
            window: None,
        };
        let mut state = Prepared::read(&fixture.store, &fixture.key, None, 1).unwrap();
        let expected = AirAces::load(source, &params, &context, &flag).unwrap();
        let actual = state
            .manifest(
                source,
                &fixture.store,
                &fixture.key,
                &params,
                &context,
                &flag,
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        for (window, _) in source
            .replay_ref()
            .index
            .iter()
            .filter(|row| row.branch == 1)
            .enumerate()
        {
            let params = AirAcesParams {
                branch_id: "1".into(),
                view: AirAcesView::Window,
                window: Some(window),
            };
            let expected = AirAces::load(source, &params, &context, &flag).unwrap();
            let actual = series::read_reply::<AirAcesWindowReply>(
                &fixture.store,
                &fixture.key,
                "air-aces-1-windows",
                window as u64,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
        let state = Prepared::read(&fixture.store, &fixture.key, None, 1).unwrap();
        for hour in [10, 11, 12, 13, 14] {
            let params = AirAcePortraitParams {
                branch_id: "1".into(),
                hour,
                ids: vec!["1".into(), "2".into(), "999".into()],
            };
            let expected = AirAcePortraits::load(source, &params, &context, &flag).unwrap();
            let actual = portrait_reply(source, &params, &context, &flag, |id, hour| {
                state.portrait_at(id, hour)
            })
            .unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
    }

    #[test]
    fn prepared_aces_windows_and_portraits_match_after_two_commits_resume_and_append() {
        let mut fixture = Fixture::new();
        let mut live = super::super::tests::ace(1, true);
        live.assigned_wing = 70;
        let first = vec![super::super::tests::frame(
            10,
            vec![live.clone(), super::super::tests::ace(2, false)],
        )];
        let mut dead = live.clone();
        dead.alive = 0;
        dead.death = Some(AceDeath {
            killer_ace: 2,
            killer_country: 11,
        });
        let second = vec![super::super::tests::frame(
            11,
            vec![dead.clone(), super::super::tests::ace(2, false)],
        )];
        let original = records(1, first.clone());
        let first_replay =
            Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&original)).unwrap();
        let continued = continuation_records(&first_replay, 1, second.clone());
        let two = source(
            Replay::from_records(CAMPAIGN.into(), &[original.clone(), continued.clone()]).unwrap(),
        );
        assert_eq!(two.replay_ref().index.len(), 2);
        let mut state = Prepared::default();
        let mut map = dashboard_base::prepared_map::Builder::new(
            NonZeroU32::new(24).unwrap(),
            dashboard_base::prepared_map::selected_tables(),
            Default::default(),
        )
        .unwrap();
        let mut writer = fixture.writer();
        publish(&mut state, &mut writer, &mut map, &two, 0);
        writer.commit().unwrap();
        state = Prepared::resume(&fixture.store, &fixture.key, None, two.replay_ref(), 1).unwrap();
        let mut writer = fixture.writer();
        publish(&mut state, &mut writer, &mut map, &two, 1);
        writer.commit().unwrap();
        assert_parity(&fixture, &two);
        let mut renamed =
            super::super::tests::frame(13, vec![dead.clone(), super::super::tests::ace(2, false)]);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *renamed.tables[TableId::Countries.index()]
        {
            rows.iter_mut().find(|row| row.id == 9001).unwrap().tag = b"renamed owner".to_vec();
        }
        if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
            &mut *renamed.tables[TableId::VariantDefinitions.index()]
        {
            rows[0].display_name = b"Renamed design".to_vec();
        }
        let tail = vec![
            renamed,
            super::super::tests::frame(14, vec![dead, super::super::tests::ace(2, false)]),
        ];
        let appended = continuation_records(two.replay_ref(), 1, tail.clone());
        let full = source(
            Replay::from_records(CAMPAIGN.into(), &[original, continued, appended]).unwrap(),
        );
        let previous = fixture.key.clone();
        let mut writer = fixture.append_writer();
        state = Prepared::resume(
            &fixture.store,
            &fixture.key,
            Some(&previous),
            full.replay_ref(),
            1,
        )
        .unwrap();
        publish(&mut state, &mut writer, &mut map, &full, 2);
        writer.commit().unwrap();
        assert_parity(&fixture, &full);
        assert!(
            state.portrait_at(1, 12).is_none(),
            "a recording gap cannot supply a portrait"
        );
    }

    #[test]
    fn aces_deferred_dates_survive_two_commits_resume_and_an_appended_death() {
        let mut fixture = Fixture::new();
        let live = super::super::tests::frame(10, vec![super::super::tests::ace(1, true)]);
        let frames = |first, last| {
            (first..=last)
                .map(|hour| {
                    let mut frame = live.clone();
                    frame.hour = hour;
                    frame
                })
                .collect::<Vec<_>>()
        };
        let first = frames(10, 19);
        let second = frames(20, 29);
        let original = records(1, first.clone());
        let prefix =
            Replay::from_records(CAMPAIGN.into(), std::slice::from_ref(&original)).unwrap();
        let continued = continuation_records(&prefix, 1, second.clone());
        let two = source(
            Replay::from_records(CAMPAIGN.into(), &[original.clone(), continued.clone()]).unwrap(),
        );
        let mut state = Prepared::default();
        let mut map = dashboard_base::prepared_map::Builder::new(
            NonZeroU32::new(24).unwrap(),
            &TableId::ALL,
            Default::default(),
        )
        .unwrap();
        for (block, _chunk) in [&first[..], &second[..]].into_iter().enumerate() {
            let mut writer = fixture.writer();
            publish(&mut state, &mut writer, &mut map, &two, block);
            writer.commit().unwrap();
            state =
                Prepared::resume(&fixture.store, &fixture.key, None, two.replay_ref(), 1).unwrap();
        }
        assert_parity(&fixture, &two);
        let mut tail = frames(30, 39);
        let mut dead = live.clone();
        dead.hour = 40;
        if let TableView::Complete(TableRows::AirAces(rows)) =
            &mut *dead.tables[TableId::AirAces.index()]
        {
            rows[0].alive = 0;
            rows[0].death = Some(AceDeath {
                killer_ace: 2,
                killer_country: 11,
            });
        }
        tail.push(dead);
        let appended = continuation_records(two.replay_ref(), 1, tail.clone());
        let full = source(
            Replay::from_records(CAMPAIGN.into(), &[original, continued, appended]).unwrap(),
        );
        let previous = fixture.key.clone();
        let mut writer = fixture.append_writer();
        state = Prepared::resume(
            &fixture.store,
            &fixture.key,
            Some(&previous),
            full.replay_ref(),
            1,
        )
        .unwrap();
        publish(&mut state, &mut writer, &mut map, &full, 2);
        writer.commit().unwrap();
        assert_eq!(state.scan.aces[0].last_alive, Some(39));
        assert_eq!(state.scan.aces[0].dead_by, Some(40));
        assert_parity(&fixture, &full);
    }

    #[test]
    fn prepared_aces_retains_all_5000_country_rosters() {
        let fixture = Fixture::new();
        let mut frame = scale_frame(1, 10, 5000);
        frame.metrics.table_ns[TableId::AirAces.index()] = 1;
        let rows = (0..5000)
            .map(|id| AirAceRow {
                country: 100_000 + id,
                ..super::super::tests::ace(u64::from(id) + 1, true)
            })
            .collect();
        *frame.tables[TableId::AirAces.index()] = TableView::Complete(TableRows::AirAces(rows));
        let source = source(
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone()])]).unwrap(),
        );
        let mut state = Prepared::default();
        let mut writer = fixture.writer();
        let mut map = dashboard_base::prepared_map::Builder::new(
            NonZeroU32::new(24).unwrap(),
            dashboard_base::prepared_map::selected_tables(),
            Default::default(),
        )
        .unwrap();
        publish(&mut state, &mut writer, &mut map, &source, 0);
        writer.commit().unwrap();
        assert_eq!(state.scan.aces.len(), 5000);
        assert_eq!(state.portraits.len(), 5000);
        assert_parity(&fixture, &source);
    }

    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn prepared_ace_portrait_operands_grow_with_changes_and_not_reads() {
        let fixture = Fixture::new();
        let mut frame = super::super::tests::frame(0, vec![super::super::tests::ace(1, true)]);
        let source = source(
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone()])]).unwrap(),
        );
        let mut state = Prepared::default();
        let mut sizes = Vec::new();
        for hour in 0..10_000 {
            frame.hour = hour;
            state.observe(&frame, source.replay_ref()).unwrap();
            if matches!(hour, 99 | 9999) {
                sizes.push(serde_json::to_vec(&state.portraits).unwrap().len());
            }
        }
        assert_eq!(state.portraits[&1].len(), 1);
        assert!(sizes[1] <= sizes[0] + 4);
        eprintln!("ace portrait operands JSON bytes at 100 and 10,000 reads: {sizes:?}");
        let before = state.portrait_at(1, 9999).unwrap();
        if let TableView::Complete(TableRows::AirAces(rows)) =
            &mut *frame.tables[TableId::AirAces.index()]
        {
            rows[0].name = b"Different name".to_vec();
        }
        frame.hour = 10_000;
        state.observe(&frame, source.replay_ref()).unwrap();
        assert_eq!(state.portraits[&1].len(), 2);
        assert_ne!(state.portrait_at(1, 10_000).unwrap(), before);
        let mut writer = fixture.writer();
        state
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        writer.commit().unwrap();
        let restored = Prepared::read(&fixture.store, &fixture.key, None, 1).unwrap();
        assert_eq!(restored.portrait_at(1, 9999), Some(before));
    }
}
