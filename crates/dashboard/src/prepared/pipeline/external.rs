//! Air-statistics preparation from borrowed frames supplied outside the pipeline.

use super::{
    Batches, CHECKPOINT, Checkpoint, Refused, branches_supported, checkpoint, independent, invalid,
    last_blocks, metadata_prefix, write_checkpoint_state, writer_busy, zero_checkpoint,
};
use crate::products::air_statistics::AirStatisticsObserver;
use crate::{Source, products::ContentOptions};
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    pages::{Committed, HeldPages},
    series,
};
use dashboard_keyed::{Observer, Scan};
use replay_frame::frame::DecodedFrame;
use std::{
    io,
    num::NonZeroU32,
    sync::{Arc, atomic::AtomicBool},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    AirStatistics,
}

pub struct BoundSource<'a> {
    source: &'a Source,
    content: Option<&'a ContentOptions>,
    store: Arc<store::Store>,
    key: store::SourceKey,
}

impl<'a> BoundSource<'a> {
    pub fn open(source: &'a Source, content: Option<&'a ContentOptions>) -> io::Result<Self> {
        let (store, key) = super::super::files(source, content)?;
        let key = dashboard_base::prepared::refresh_saved_binding(source, &store, key)?;
        Ok(Self {
            source,
            content,
            store,
            key,
        })
    }

    fn saved(&self) -> io::Result<Option<store::SourceKey>> {
        let saved = self.store.saved_key()?;
        Ok(saved.filter(|previous| {
            previous == &self.key
                || previous.recording_size() < self.key.recording_size()
                || self.key.renames(previous)
        }))
    }

    fn checkpoint(
        &self,
        saved: Option<&store::SourceKey>,
    ) -> io::Result<Option<(Checkpoint, u64)>> {
        let Some(pair) = checkpoint(&self.store, &self.key, saved)? else {
            return Ok(None);
        };
        if !metadata_prefix(&pair.0, self.source.replay_ref(), self.content)?
            || !branches_supported(
                &self.store,
                &self.key,
                saved,
                self.source.replay_ref(),
                &pair.0,
            )?
        {
            return Ok(None);
        }
        Ok(Some(pair))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Publication {
    Published,
    Busy,
    Stale,
}

struct Pending {
    saved: Option<store::SourceKey>,
    baseline: Option<independent::Progress>,
    catalogue: Vec<blake3::Hash>,
    progress: independent::Progress,
    refused: Refused,
    scan: Box<Scan<AirStatisticsObserver>>,
    pages: HeldPages,
    batches: Batches,
    next_block: usize,
    reached: Option<i32>,
    bound: bool,
}

pub struct ExternalFamily<'a> {
    binding: BoundSource<'a>,
    branch: u64,
    pending: Option<Pending>,
    stopped: Publication,
}

fn catalogue_hashes(
    binding: &BoundSource<'_>,
    saved: Option<&store::SourceKey>,
    branch: u64,
) -> io::Result<Vec<blake3::Hash>> {
    let mut ids = Vec::new();
    let section = series::catalogue_section(AirStatisticsObserver::NAME, branch);
    let Some(saved) = saved else { return Ok(ids) };
    let count = if saved == &binding.key {
        binding.store.range_count(&binding.key, &section)?
    } else {
        binding
            .store
            .prefix_range_count(saved, &binding.key, &section)?
    }
    .unwrap_or(0);
    for range in 0..count {
        let mut file = if saved == &binding.key {
            binding.store.read(&binding.key, &section, range)?
        } else {
            binding
                .store
                .read_prefix(saved, &binding.key, &section, range)?
        }
        .ok_or_else(|| invalid("Prepared air-statistics catalogue is missing"))?;
        let mut hash = blake3::Hasher::new();
        hash.update_reader(&mut file)?;
        ids.push(hash.finalize());
    }
    Ok(ids)
}

impl<'a> ExternalFamily<'a> {
    pub fn begin(binding: BoundSource<'a>, family: Family, branch: u64) -> io::Result<Self> {
        let Family::AirStatistics = family;
        let name = AirStatisticsObserver::NAME;
        if !independent::registered(name) || independent::gating(name) != Some(name) {
            return Err(invalid("Air statistics are not prepared independently"));
        }
        super::install_gates();
        let mut saved = binding.saved()?;
        let _scope = binding.store.scope(&binding.key)?;
        let loaded = binding.checkpoint(saved.as_ref())?;
        let baseline = loaded
            .as_ref()
            .map(|(state, _)| state.independent[name].clone());
        let progress = baseline.clone().unwrap_or_default();
        if loaded.is_none() {
            saved = None;
        }
        let replay = binding.source.replay_ref();
        if !replay.branches.iter().any(|row| row.id == branch)
            || replay
                .index
                .get(progress.next_block)
                .is_some_and(|block| block.branch != branch)
        {
            return Err(invalid(
                "External family must continue the global recording prefix",
            ));
        }
        let refused = progress.refused.get(&branch).cloned().unwrap_or_default();
        let scan = if saved.is_some() && refused.open(name) {
            series::read_saved_catalogue::<Scan<AirStatisticsObserver>>(
                &binding.store,
                &binding.key,
                saved.as_ref(),
                name,
                branch,
            )?
            .map(|(_, scan)| scan)
        } else {
            None
        };
        let pending = Pending {
            catalogue: catalogue_hashes(&binding, saved.as_ref(), branch)?,
            saved,
            baseline,
            next_block: progress.next_block,
            progress,
            refused,
            scan: Box::new(scan.unwrap_or_default()),
            pages: HeldPages::default(),
            batches: Batches::new(replay),
            reached: None,
            bound: false,
        };
        Ok(Self {
            binding,
            branch,
            pending: Some(pending),
            stopped: Publication::Stale,
        })
    }

    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        block: usize,
        hour: i32,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let Some(pending) = &mut self.pending else {
            return Ok(());
        };
        let replay = self.binding.source.replay_ref();
        let index = replay
            .index
            .get(block)
            .ok_or_else(|| invalid("External block is not recorded"))?;
        if block != pending.next_block
            || index.branch != self.branch
            || frame.branch != self.branch
            || hour != frame.hour
            || hour < index.first_hour
            || hour > index.last_hour
            || pending.reached.is_some_and(|previous| previous >= hour)
        {
            return Err(invalid(
                "External frame position does not continue its recorded block",
            ));
        }
        if pending.refused.open(AirStatisticsObserver::NAME) {
            if !pending.bound {
                pending.scan.observer.bind(
                    self.binding.source,
                    self.binding.content,
                    &mut || Ok(()),
                )?;
                pending.bound = true;
            }
            let alone;
            let changes = if pending
                .reached
                .is_none_or(|last| last.checked_add(1) != Some(hour))
            {
                alone = FrameChanges::alone(frame, replay.format);
                &alone
            } else {
                changes
            };
            pending.refused.check(
                AirStatisticsObserver::NAME,
                hour,
                pending.scan.prepare_changed(replay.format, frame, changes),
            )?;
        }
        pending.reached = Some(hour);
        Ok(())
    }

    pub fn seal_block(&mut self, block: usize) -> io::Result<()> {
        let Some(pending) = &mut self.pending else {
            return Ok(());
        };
        let replay = self.binding.source.replay_ref();
        let index = replay
            .index
            .get(block)
            .ok_or_else(|| invalid("External block is not recorded"))?;
        if block != pending.next_block || index.branch != self.branch {
            return Err(invalid(
                "External block does not continue the global recording prefix",
            ));
        }
        if pending.refused.open(AirStatisticsObserver::NAME) {
            if !pending.bound {
                pending.scan.observer.bind(
                    self.binding.source,
                    self.binding.content,
                    &mut || Ok(()),
                )?;
            }
            if pending.reached.is_none() {
                pending.scan.unreadable_block();
            }
        }
        let continues = replay
            .index
            .get(block + 1)
            .is_some_and(|next| next.branch == self.branch);
        // Held pages are earlier flushed batches, not this batch's byte budget.
        if dashboard_base::prepared::pages::publication_due(
            continues,
            false,
            pending.batches.full(block),
            || (pending.scan.buffered_bytes(), 0),
        ) {
            if pending.refused.open(AirStatisticsObserver::NAME) {
                let mut pages = pending.pages.view(Committed {
                    store: &self.binding.store,
                    key: &self.binding.key,
                    saved: pending.saved.as_ref(),
                });
                let result = pending
                    .scan
                    .flush(&mut pages, self.branch, &AtomicBool::new(false));
                pending.refused.check(
                    AirStatisticsObserver::NAME,
                    pending.reached.unwrap_or(index.last_hour),
                    result,
                )?;
            }
            pending.progress.next_block = block + 1;
            if !pending.refused.0.is_empty() {
                pending
                    .progress
                    .refused
                    .insert(self.branch, pending.refused.clone());
            }
        }
        pending.next_block = block + 1;
        pending.reached = None;
        pending.bound = false;
        Ok(())
    }

    pub fn pending_bytes(&self) -> usize {
        self.pending.as_ref().map_or(0, |pending| {
            pending
                .pages
                .bytes()
                .saturating_add(pending.scan.buffered_bytes())
        })
    }

    /// Call at most once per recorder save; only completed batches advance the saved prefix.
    /// Held pages span one recorder-save interval alongside the live scan.
    pub fn publish_saved(&mut self) -> io::Result<Publication> {
        self.publish_saved_checked(|_| Ok(()))
    }

    fn publish_saved_checked(
        &mut self,
        before_manifest: impl FnOnce(&mut store::Writer) -> io::Result<()>,
    ) -> io::Result<Publication> {
        let Some(mut pending) = self.pending.take() else {
            return Ok(self.stopped);
        };
        self.binding.key = dashboard_base::prepared::refresh_saved_binding(
            self.binding.source,
            &self.binding.store,
            self.binding.key.clone(),
        )?;
        let saved = self.binding.saved()?;
        let saved = if self.binding.checkpoint(saved.as_ref())?.is_some() {
            saved
        } else {
            None
        };
        let mut writer = match match saved.as_ref() {
            Some(previous) if previous != &self.binding.key => self
                .binding
                .store
                .append(previous, self.binding.key.clone()),
            Some(_) => self.binding.store.extend(&self.binding.key),
            None => self.binding.store.replace(self.binding.key.clone()),
        } {
            Ok(writer) => writer,
            Err(error) if writer_busy(&error) => {
                self.stopped = Publication::Busy;
                return Ok(self.stopped);
            }
            Err(error) => {
                if self.binding.store.saved_key()? != saved {
                    self.stopped = Publication::Stale;
                    return Ok(self.stopped);
                }
                return Err(error);
            }
        };
        if saved.is_none()
            && self
                .binding
                .checkpoint(self.binding.saved()?.as_ref())?
                .is_some()
        {
            self.stopped = Publication::Stale;
            return Ok(self.stopped);
        }
        let current = writer.read_at(CHECKPOINT, 0)?;
        let current = current
            .map(|file| {
                let bytes = file.len();
                let state: Checkpoint = serde_json::from_reader(io::BufReader::new(file))?;
                let bytes = state
                    .bytes
                    .checked_add(bytes)
                    .ok_or_else(|| invalid("Prepared byte count overflow"))?;
                Ok::<_, io::Error>((state, bytes))
            })
            .transpose()?;
        let mut ids = Vec::new();
        let section = series::catalogue_section(AirStatisticsObserver::NAME, self.branch);
        while let Some(mut file) = writer.read_at(&section, ids.len() as u64)? {
            let mut hash = blake3::Hasher::new();
            hash.update_reader(&mut file)?;
            ids.push(hash.finalize());
        }
        let baseline = current
            .as_ref()
            .and_then(|(state, _)| state.independent.get(AirStatisticsObserver::NAME))
            .cloned();
        if baseline != pending.baseline || ids != pending.catalogue {
            self.stopped = Publication::Stale;
            return Ok(self.stopped);
        }
        let replay = self.binding.source.replay_ref();
        let fresh = current.is_none();
        let (mut state, bytes) = match current {
            Some(pair)
                if pair.0.schema == super::SCHEMA
                    && metadata_prefix(&pair.0, replay, self.binding.content)? =>
            {
                pair
            }
            Some(_) => {
                self.stopped = Publication::Stale;
                return Ok(self.stopped);
            }
            None => (zero_checkpoint(replay, self.binding.content)?, 0),
        };
        let mut map = dashboard_base::prepared_map::Builder::new(
            NonZeroU32::new(dashboard_base::prepared_map::CHECKPOINT_STRIDE)
                .ok_or_else(|| invalid("Zero checkpoint stride"))?,
            dashboard_base::prepared_map::selected_tables(),
            state.map.clone(),
        )?;
        map.set_ranges(replay);
        if fresh {
            map.save_gaps(&mut writer, &replay.gaps)?;
            state.map = map.progress().clone();
        }
        pending.pages.write(&mut writer)?;
        state
            .independent
            .insert(AirStatisticsObserver::NAME.into(), pending.progress.clone());
        let cancelled = AtomicBool::new(false);
        let context = crate::products::LoadContext {
            content: self.binding.content.cloned(),
        };
        let last_blocks = last_blocks(replay);
        let batches = &pending.batches;
        let phase = independent::Phase {
            source: self.binding.source,
            replay,
            store: &self.binding.store,
            key: &self.binding.key,
            context: &context,
            ranges: &map.progress().ranges,
            last_blocks: &last_blocks,
            batches,
            measured: false,
            cancelled: &cancelled,
        };
        independent::save_coverage(
            &mut writer,
            &phase,
            AirStatisticsObserver::NAME,
            &pending.progress,
            self.branch,
        )?;
        write_checkpoint_state(&mut writer, &mut state, replay, bytes)?;
        before_manifest(&mut writer)?;
        writer.commit()?;
        pending.saved = Some(self.binding.key.clone());
        pending.baseline = Some(pending.progress.clone());
        pending.catalogue = catalogue_hashes(&self.binding, pending.saved.as_ref(), self.branch)?;
        self.pending = Some(pending);
        Ok(Publication::Published)
    }

    pub fn abort(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Accepted, State, prepass, run};
    use super::*;
    use crate::test_fixtures::{CAMPAIGN, continuation_records, records, world_frame};
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use replay::{container::Replay, spool::Record};
    use replay_data::rows::{AirStatisticsRow, TableId, TableRows};
    use replay_frame::{branches::Gap, frame::TableView};
    use std::{
        collections::BTreeMap,
        io::{Read, Seek, SeekFrom, Write},
        os::windows::fs::OpenOptionsExt,
        path::PathBuf,
        sync::{Mutex, atomic::Ordering, mpsc},
        time::Duration,
    };

    type Payloads = BTreeMap<(String, u64), Vec<u8>>;

    struct Fixture {
        root: PathBuf,
        path: PathBuf,
        store: Arc<store::Store>,
    }

    impl Fixture {
        fn new(replay: &Replay) -> Self {
            let root =
                std::env::temp_dir().join(format!("external-family-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let path = root.join("campaign.battleplan");
            std::fs::write(&path, replay.bytes()).unwrap();
            let store = super::super::super::recording_store(&path).unwrap();
            Self { root, path, store }
        }

        fn source(&self) -> Arc<Source> {
            Arc::new(
                Source::open(
                    crate::SourceKey {
                        source_id: "external-family".into(),
                        revision: crate::protocol::replay_revision(&self.path).unwrap(),
                    },
                    self.path.clone(),
                    &AtomicBool::new(false),
                )
                .unwrap(),
            )
        }

        fn replace(&self, replay: &Replay) {
            let candidate = self.root.join("candidate");
            std::fs::write(&candidate, replay.bytes()).unwrap();
            replay::publication::replace_file(&candidate, &self.path).unwrap();
        }

        fn unreadable(&self, replay: &Replay, block: usize) {
            let index = &replay.index[block];
            let sections = replay.block_sections(index).unwrap().unwrap();
            let stored: u64 = sections.iter().map(|section| section.compressed).sum();
            let mut offset = index.offset + index.length - stored;
            for section in sections {
                if section.section == replay::container::Section::Diagnostics {
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .open(&self.path)
                        .unwrap();
                    let modified = file.metadata().unwrap().modified().unwrap();
                    file.seek(SeekFrom::Start(offset)).unwrap();
                    file.write_all(&vec![0; section.compressed as usize])
                        .unwrap();
                    file.set_times(std::fs::FileTimes::new().set_modified(modified))
                        .unwrap();
                    return;
                }
                offset += section.compressed;
            }
            panic!("synthetic block has no diagnostics");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.store.clear().unwrap();
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    fn frame(hour: i32) -> DecodedFrame {
        let mut frame = world_frame(hour, false);
        if hour >= 103 {
            if let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                countries
                    .iter_mut()
                    .find(|country| country.id == 9001)
                    .unwrap()
                    .tag = b"MOD".to_vec();
            }
        }
        *frame.tables[TableId::AirStatistics.index()] = if hour == 102 {
            TableView::Unavailable {
                reason: "synthetic unknown hour".into(),
            }
        } else {
            TableView::Complete(TableRows::AirStatistics(vec![AirStatisticsRow {
                country: 11,
                region: 3,
                year_slot: u32::from(hour >= 106),
                year_window_wrapped: u8::from(hour >= 106),
                channels: [i64::from(hour); 24],
                ..Default::default()
            }]))
        };
        frame.metrics.table_ns[TableId::AirStatistics.index()] = 1;
        frame
    }

    fn scenario() -> (Replay, Vec<Vec<DecodedFrame>>) {
        let blocks: Vec<Vec<_>> = [100, 102, 106, 108, 110]
            .into_iter()
            .map(|first| vec![frame(first), frame(first + 1)])
            .collect();
        let mut replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, blocks[0].clone())]).unwrap();
        for (block, frames) in blocks.iter().enumerate().skip(1) {
            let mut input = continuation_records(&replay, 1, frames.clone());
            if block == 2 {
                input.insert(
                    2,
                    Record::Gap(Gap {
                        branch: 1,
                        first_hour: 104,
                        last_hour: 105,
                        reason: "synthetic missing hours".into(),
                    }),
                );
            }
            replay = replay.append(&[input]).unwrap();
        }
        assert_eq!(replay.index.len(), blocks.len());
        (replay, blocks)
    }

    fn begin<'a>(source: &'a Source, content: Option<&'a ContentOptions>) -> ExternalFamily<'a> {
        ExternalFamily::begin(
            BoundSource::open(source, content).unwrap(),
            Family::AirStatistics,
            1,
        )
        .unwrap()
    }

    fn feed(family: &mut ExternalFamily<'_>, blocks: &[Vec<DecodedFrame>], through: usize) {
        let start = family.pending.as_ref().unwrap().next_block;
        for (block, frames) in blocks.iter().enumerate().take(through).skip(start) {
            let mut changes = FrameChangeTracker::new();
            for frame in frames {
                let changes = changes.observe(frame, family.binding.source.replay_ref().format);
                family.observe(frame, block, frame.hour, &changes).unwrap();
            }
            family.seal_block(block).unwrap();
        }
    }

    fn payloads(source: &Source, content: Option<&ContentOptions>) -> Payloads {
        let binding = BoundSource::open(source, content).unwrap();
        let Some(listing) =
            store::Listing::open(&binding.store.directory().join("manifest")).unwrap()
        else {
            return BTreeMap::new();
        };
        let session = binding.store.session(&binding.key).unwrap().unwrap();
        let mut result = BTreeMap::new();
        for (name, count) in listing.sections() {
            for range in 0..count {
                let mut bytes = Vec::new();
                session
                    .read(name, range)
                    .unwrap()
                    .unwrap()
                    .read_to_end(&mut bytes)
                    .unwrap();
                result.insert((name.to_owned(), range), bytes);
            }
        }
        result
    }

    fn air(payloads: &Payloads) -> Payloads {
        payloads
            .iter()
            .filter(|((section, _), _)| {
                section.starts_with("air-statistics-")
                    || section.starts_with("preparation-family-air-statistics-")
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    }

    fn state(source: &Source, content: Option<&ContentOptions>) -> Checkpoint {
        let binding = BoundSource::open(source, content).unwrap();
        binding
            .checkpoint(binding.saved().unwrap().as_ref())
            .unwrap()
            .unwrap()
            .0
    }

    fn ordinary(source: &Source, content: Option<&ContentOptions>) -> Accepted {
        independent::with_test_one_after_another(|| {
            prepass::with_test_prepass(1, None, || {
                run(source, content, &AtomicBool::new(false), |_| {})
            })
        })
        .unwrap()
    }

    fn without_replanning<T>(run: impl FnOnce() -> T) -> T {
        crate::prepared::with_test_cell(&super::super::TEST_REPLANS, Some(0), || {
            let result = run();
            assert_eq!(super::super::TEST_REPLANS.get(), Some(0));
            result
        })
    }

    fn ordinary_to(source: &Source, content: Option<&ContentOptions>, frontier: usize) {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancelled);
        let result = independent::with_test_one_after_another(|| {
            independent::with_test_on_publish(
                move |name, next| {
                    if name == AirStatisticsObserver::NAME && next == frontier {
                        stop.store(true, Ordering::Release);
                    }
                },
                || prepass::with_test_prepass(1, None, || run(source, content, &cancelled, |_| {})),
            )
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(
            state(source, content).independent[AirStatisticsObserver::NAME].next_block,
            frontier
        );
    }

    fn assert_metadata_equal(actual: &Checkpoint, expected: &Checkpoint) {
        macro_rules! same {
            ($($field:ident),+ $(,)?) => { $(
                assert_eq!(serde_json::to_vec(&actual.$field).unwrap(), serde_json::to_vec(&expected.$field).unwrap(), stringify!($field));
            )+ };
        }
        same!(
            schema, selected, header, index, sessions, branches, gaps, content
        );
        assert_eq!(
            serde_json::to_vec(&actual.independent[AirStatisticsObserver::NAME]).unwrap(),
            serde_json::to_vec(&expected.independent[AirStatisticsObserver::NAME]).unwrap()
        );
        assert_eq!(actual.map.selected, expected.map.selected);
        assert_eq!(actual.map.stride, expected.map.stride);
        assert_eq!(
            serde_json::to_vec(&actual.map.ranges).unwrap(),
            serde_json::to_vec(&expected.map.ranges).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&actual.map.gaps).unwrap(),
            serde_json::to_vec(&expected.map.gaps).unwrap()
        );
    }

    #[test]
    fn supplied_frames_match_ordinary_raw_pages_progress_and_binding_at_each_frontier() {
        let (replay, blocks) = scenario();
        let reference = Fixture::new(&replay);
        let external = Fixture::new(&replay);
        let source = external.source();
        let mut family = begin(&source, None);
        for frontier in [1, 2, 4, 5] {
            let ordinary_source = reference.source();
            reference.store.clear().unwrap();
            ordinary_to(&ordinary_source, None, frontier);
            feed(&mut family, &blocks, frontier);
            assert_eq!(family.publish_saved().unwrap(), Publication::Published);
            assert_eq!(
                air(&payloads(&source, None)),
                air(&payloads(&ordinary_source, None)),
                "frontier {frontier}"
            );
            let actual = state(&source, None);
            assert_metadata_equal(&actual, &state(&ordinary_source, None));
            assert_eq!(actual.cursor.next_block, 0);
            assert!(actual.cursor.previous.is_none() && actual.map.branches.is_empty());
            assert_eq!(actual.independent.len(), independent::names().len());
            assert!(
                actual
                    .independent
                    .iter()
                    .all(|(name, progress)| name == AirStatisticsObserver::NAME
                        || *progress == independent::Progress::default())
            );
            if frontier == 2 {
                family.abort();
                family = begin(&source, None);
            }
        }
    }

    #[test]
    fn a_saved_family_reopens_after_recording_growth_with_current_family_coverage() {
        let (replay, blocks) = scenario();
        let first =
            Replay::from_records(CAMPAIGN.into(), &[records(1, blocks[0].clone())]).unwrap();
        let fixture = Fixture::new(&first);
        let source = fixture.source();
        let mut family = begin(&source, None);
        feed(&mut family, &blocks[..1], 1);
        assert_eq!(family.publish_saved().unwrap(), Publication::Published);
        let map = serde_json::to_vec(&state(&source, None).map).unwrap();
        drop(family);
        fixture.replace(&replay);
        let grown = fixture.source();
        let mut family = begin(&grown, None);
        assert_eq!(family.pending.as_ref().unwrap().next_block, 1);
        feed(&mut family, &blocks, blocks.len());
        assert_eq!(family.publish_saved().unwrap(), Publication::Published);
        assert_eq!(serde_json::to_vec(&state(&grown, None).map).unwrap(), map);
        let external = air(&payloads(&grown, None));
        ordinary(&grown, None);
        let finished = payloads(&grown, None);
        drop(family);
        fixture.store.clear().unwrap();
        let other = fixture.source();
        ordinary(&other, None);
        let expected = payloads(&other, None);
        assert_eq!(external, air(&expected));
        assert_eq!(finished, expected);
    }

    #[test]
    fn unreadable_blocks_leave_the_last_complete_family_prefix_identical() {
        let (replay, blocks) = scenario();
        let reference = Fixture::new(&replay);
        let other = reference.source();
        ordinary_to(&other, None, 2);
        let external = Fixture::new(&replay);
        let source = external.source();
        let mut family = begin(&source, None);
        feed(&mut family, &blocks, 2);
        assert_eq!(family.publish_saved().unwrap(), Publication::Published);
        let before = air(&payloads(&source, None));
        assert_eq!(before, air(&payloads(&other, None)));
        reference.unreadable(&replay, 3);
        external.unreadable(&replay, 3);
        assert!(run(&other, None, &AtomicBool::new(false), |_| {}).is_err());
        let consumer = source.replay_ref().section_consumer(2);
        consumer.at(3);
        let failed = source.replay_ref().read_block_alone(
            &source.replay_ref().index[3],
            AirStatisticsObserver::TABLES,
            |_, _| panic!("an unreadable block supplies no frame"),
            || Ok(()),
        );
        assert!(failed.is_err());
        family.abort();
        assert_eq!(air(&payloads(&source, None)), before);
        assert_eq!(air(&payloads(&other, None)), before);
        assert_metadata_equal(&state(&source, None), &state(&other, None));
    }

    #[test]
    fn sealing_a_block_without_frames_matches_the_ordinary_keyed_branch_lifecycle() {
        use super::super::independent::Own;
        let (replay, blocks) = scenario();
        let reference = Fixture::new(&replay);
        let other = reference.source();
        ordinary_to(&other, None, 1);
        let binding = BoundSource::open(&other, None).unwrap();
        let mut branch = independent::keyed::AIR_STATISTICS
            .load(
                &independent::Load {
                    store: &binding.store,
                    key: &binding.key,
                    saved: Some(&binding.key),
                    replay: other.replay_ref(),
                },
                1,
            )
            .unwrap();
        branch.begin_block();
        branch.end_block(other.replay_ref(), 1);
        let mut writer = binding.store.extend(&binding.key).unwrap();
        branch
            .flush(&mut writer, other.replay_ref(), 1, &AtomicBool::new(false))
            .unwrap();
        writer.commit().unwrap();
        let fixture = Fixture::new(&replay);
        let source = fixture.source();
        let mut family = begin(&source, None);
        feed(&mut family, &blocks, 1);
        assert_eq!(family.publish_saved().unwrap(), Publication::Published);
        family.seal_block(1).unwrap();
        assert_eq!(family.publish_saved().unwrap(), Publication::Published);
        let pages = |source: &Source| -> Payloads {
            air(&payloads(source, None))
                .into_iter()
                .filter(|((name, _), _)| name.starts_with("air-statistics-"))
                .collect()
        };
        assert_eq!(pages(&source), pages(&other));
    }

    #[test]
    fn save_inside_an_ordinary_batch_publishes_only_its_completed_frontier() {
        let (replay, blocks) = scenario();
        let fixture = Fixture::new(&replay);
        let source = fixture.source();
        let mut family = begin(&source, None);
        feed(&mut family, &blocks, 3);
        assert_eq!(family.publish_saved().unwrap(), Publication::Published);
        assert_eq!(
            state(&source, None).independent[AirStatisticsObserver::NAME].next_block,
            2
        );
        feed(&mut family, &blocks, blocks.len());
        assert_eq!(family.publish_saved().unwrap(), Publication::Published);
        let reference = Fixture::new(&replay);
        let reference_source = reference.source();
        ordinary(&reference_source, None);
        assert_eq!(
            air(&payloads(&source, None)),
            air(&payloads(&reference_source, None))
        );
    }

    #[test]
    fn held_earlier_batches_do_not_move_the_next_ordinary_byte_boundary() {
        use dashboard_base::prepared::pages::Pages;
        let (replay, blocks) = scenario();
        let fixture = Fixture::new(&replay);
        let source = fixture.source();
        let mut family = begin(&source, None);
        feed(&mut family, &blocks, 2);
        let pending = family.pending.as_mut().unwrap();
        pending
            .pages
            .view(Committed {
                store: &family.binding.store,
                key: &family.binding.key,
                saved: None,
            })
            .write_at(
                "synthetic-earlier-batch",
                0,
                &vec![0; super::super::BATCH_BYTES],
            )
            .unwrap();
        feed(&mut family, &blocks, 3);
        assert_eq!(family.pending.as_ref().unwrap().next_block, 3);
        assert_eq!(family.pending.as_ref().unwrap().progress.next_block, 2);
        assert!(family.pending_bytes() >= super::super::BATCH_BYTES);
        family.abort();
        assert_eq!(family.pending_bytes(), 0);
    }

    #[test]
    fn a_scan_byte_budget_crossing_matches_the_ordinary_raw_pages_and_progress() {
        let (replay, blocks) = scenario();
        let fixture = Fixture::new(&replay);
        let reference = Fixture::new(&replay);
        let source = fixture.source();
        let ordinary_source = reference.source();
        crate::prepared::with_test_cell(
            &dashboard_base::prepared::pages::TEST_BATCH_BYTES,
            1,
            || {
                let mut family = begin(&source, None);
                feed(&mut family, &blocks, 2);
                let mut changes = FrameChangeTracker::new();
                for frame in &blocks[2] {
                    let changes = changes.observe(frame, replay.format);
                    family.observe(frame, 2, frame.hour, &changes).unwrap();
                }
                let pending = family.pending.as_ref().unwrap();
                assert!(!pending.batches.full(2));
                assert!(replay.index.get(3).is_some());
                assert!(pending.scan.buffered_bytes() >= 1);
                assert_eq!(
                    family.pending_bytes(),
                    pending.pages.bytes() + pending.scan.buffered_bytes()
                );
                family.seal_block(2).unwrap();
                assert_eq!(family.pending.as_ref().unwrap().progress.next_block, 3);
                assert_eq!(family.publish_saved().unwrap(), Publication::Published);
                ordinary_to(&ordinary_source, None, 3);
                assert_eq!(
                    air(&payloads(&source, None)),
                    air(&payloads(&ordinary_source, None))
                );
                assert_metadata_equal(&state(&source, None), &state(&ordinary_source, None));
            },
        );
    }

    #[test]
    fn an_unchanged_checkpoint_never_replans_including_after_a_growth_commit() {
        let (replay, blocks) = scenario();
        for growth in [false, true] {
            let first =
                Replay::from_records(CAMPAIGN.into(), &[records(1, blocks[0].clone())]).unwrap();
            let fixture = Fixture::new(if growth { &first } else { &replay });
            let source = fixture.source();
            if growth {
                ordinary(&source, None);
                fixture.replace(&replay);
            } else {
                ordinary_to(&source, None, 2);
            }
            let source = fixture.source();
            without_replanning(|| {
                ordinary(&source, None);
                assert_eq!(ordinary(&source, None).new_blocks, 0);
            });
        }
    }

    #[test]
    fn normal_preparation_after_external_publication_skips_its_prefix_and_finishes_identically() {
        let (replay, blocks) = scenario();
        let fixture = Fixture::new(&replay);
        let reference_source = fixture.source();
        ordinary(&reference_source, None);
        let expected = payloads(&reference_source, None);
        fixture.store.clear().unwrap();
        for frontier in [2, blocks.len()] {
            let source = fixture.source();
            let previous = store::SourceKey::capture(&fixture.path, &[]).unwrap();
            fixture.store.replace(previous).unwrap().commit().unwrap();
            let app = fixture.source();
            super::super::super::files(&app, None).unwrap();
            fixture.store.clear().unwrap();
            let mut family = begin(&source, None);
            feed(&mut family, &blocks, frontier);
            assert_eq!(family.publish_saved().unwrap(), Publication::Published);
            let accepted = air(&payloads(&source, None));
            let observed = Arc::new(Mutex::new(Vec::new()));
            let seen = Arc::clone(&observed);
            let hook: prepass::Hook = Arc::new(move |name, frame| {
                if name == AirStatisticsObserver::NAME {
                    seen.lock().unwrap().push(frame.hour);
                }
                prepass::Fault::Continue
            });
            without_replanning(|| {
                prepass::with_test_prepass(1, Some(hook), || {
                    run(&app, None, &AtomicBool::new(false), |_| {})
                })
                .unwrap();
            });
            assert!(
                observed
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|hour| *hour > blocks[frontier - 1].last().unwrap().hour)
            );
            let finished = payloads(&source, None);
            assert_eq!(finished, expected, "frontier {frontier}");
            for (key, bytes) in accepted
                .iter()
                .filter(|((section, _), _)| section.starts_with("air-statistics-series-"))
            {
                assert_eq!(finished.get(key), Some(bytes));
            }
            let complete = state(&source, None);
            assert!(complete.forward_complete(blocks.len()));
            if frontier == blocks.len() {
                assert_eq!(air(&finished), accepted);
            }
            drop(family);
            fixture.store.clear().unwrap();
        }
    }

    #[test]
    fn staged_writes_failure_abort_and_drop_leave_only_the_previous_complete_manifest() {
        let (replay, blocks) = scenario();
        for prior in [0, 1] {
            let fixture = Fixture::new(&replay);
            let source = fixture.source();
            let mut family = begin(&source, None);
            if prior != 0 {
                feed(&mut family, &blocks, prior);
                assert_eq!(family.publish_saved().unwrap(), Publication::Published);
            }
            let before = payloads(&source, None);
            feed(&mut family, &blocks, 2);
            let error = family
                .publish_saved_checked(|writer| {
                    assert!(writer.bytes_written() > 0);
                    assert!(writer.read_at(CHECKPOINT, 0)?.is_some());
                    Err(io::Error::other(
                        "synthetic failure before manifest publication",
                    ))
                })
                .unwrap_err();
            assert!(error.to_string().contains("before manifest"));
            assert!(family.pending.is_none());
            drop(family);
            let reopened = fixture.source();
            assert_eq!(payloads(&reopened, None), before);
            for abort in [true, false] {
                let mut family = begin(&reopened, None);
                feed(&mut family, &blocks, 2);
                if abort {
                    family.abort();
                }
                drop(family);
                assert_eq!(payloads(&fixture.source(), None), before);
            }
        }
    }

    #[test]
    fn busy_discards_pending_state_and_stops_even_after_the_app_releases_its_writer() {
        let (replay, blocks) = scenario();
        let fixture = Fixture::new(&replay);
        let source = fixture.source();
        let mut family = begin(&source, None);
        feed(&mut family, &blocks, 2);
        let binding = BoundSource::open(&source, None).unwrap();
        let app = binding.store.replace(binding.key.clone()).unwrap();
        assert_eq!(family.publish_saved().unwrap(), Publication::Busy);
        assert!(family.pending.is_none());
        drop(app);
        family
            .observe(
                &frame(-1),
                usize::MAX,
                -1,
                &FrameChanges::alone(&frame(-1), replay.format),
            )
            .unwrap();
        family.seal_block(usize::MAX).unwrap();
        assert_eq!(family.publish_saved().unwrap(), Publication::Busy);
        ordinary(&source, None);
        let finished = payloads(&source, None);
        drop(family);
        fixture.store.clear().unwrap();
        let other = fixture.source();
        ordinary(&other, None);
        assert_eq!(finished, payloads(&other, None));
    }

    #[test]
    fn changed_family_baseline_discards_the_next_publication_and_stops() {
        let (replay, blocks) = scenario();
        let fixture = Fixture::new(&replay);
        let source = fixture.source();
        let mut family = begin(&source, None);
        feed(&mut family, &blocks, 1);
        assert_eq!(family.publish_saved().unwrap(), Publication::Published);
        feed(&mut family, &blocks, 2);
        ordinary_to(&source, None, 2);
        let before = payloads(&source, None);
        assert_eq!(family.publish_saved().unwrap(), Publication::Stale);
        assert!(family.pending.is_none());
        family.seal_block(usize::MAX).unwrap();
        assert_eq!(family.publish_saved().unwrap(), Publication::Stale);
        assert_eq!(payloads(&source, None), before);
    }

    #[test]
    fn external_publication_preserves_sibling_progress_map_state_and_refusals() {
        let (replay, blocks) = scenario();
        let fixture = Fixture::new(&replay);
        let source = fixture.source();
        ordinary_to(&source, None, 1);
        let before = state(&source, None);
        let before_pages = payloads(&source, None);
        let mut family = begin(&source, None);
        feed(&mut family, &blocks, 2);
        assert_eq!(family.publish_saved().unwrap(), Publication::Published);
        let after = state(&source, None);
        macro_rules! same { ($($field:ident),+ $(,)?) => { $(
            assert_eq!(serde_json::to_vec(&after.$field).unwrap(), serde_json::to_vec(&before.$field).unwrap());
        )+ }; }
        same!(
            cursor,
            map,
            refused,
            newest,
            display_error,
            display_failures
        );
        for (name, progress) in &before.independent {
            if name != AirStatisticsObserver::NAME {
                assert_eq!(after.independent[name], *progress);
            }
        }
        let after_pages = payloads(&source, None);
        for (key, bytes) in before_pages.iter().filter(|((name, _), _)| {
            name != CHECKPOINT
                && name != super::super::MAP_PROGRESS
                && !name.starts_with("air-statistics-")
                && !name.starts_with("preparation-family-air-statistics-")
        }) {
            assert_eq!(after_pages.get(key), Some(bytes));
        }
    }

    #[test]
    fn invalid_positions_and_unobserved_other_branches_cannot_advance_the_global_prefix() {
        let (replay, blocks) = scenario();
        let replay = replay
            .append(&[records(
                2,
                vec![{
                    let mut other = frame(10);
                    other.branch = 2;
                    other
                }],
            )])
            .unwrap();
        let fixture = Fixture::new(&replay);
        let source = fixture.source();
        assert!(
            ExternalFamily::begin(
                BoundSource::open(&source, None).unwrap(),
                Family::AirStatistics,
                2
            )
            .is_err()
        );
        let mut family = begin(&source, None);
        assert!(family.seal_block(1).is_err());
        let first = &blocks[0][0];
        let changes = FrameChanges::alone(first, replay.format);
        assert!(family.observe(first, 0, first.hour + 1, &changes).is_err());
        family.observe(first, 0, first.hour, &changes).unwrap();
        assert!(family.observe(first, 0, first.hour, &changes).is_err());
        assert!(family.observe(&frame(99), 0, 99, &changes).is_err());
        family.abort();
        let mut family = begin(&source, None);
        feed(&mut family, &blocks, blocks.len());
        assert!(family.seal_block(blocks.len()).is_err());
        assert_eq!(family.publish_saved().unwrap(), Publication::Published);
        assert_eq!(
            state(&source, None).independent[AirStatisticsObserver::NAME].next_block,
            blocks.len()
        );
    }

    #[test]
    fn app_waits_visibly_for_another_holder_then_finishes_without_failure() {
        let (replay, _) = scenario();
        let fixture = Fixture::new(&replay);
        let source = fixture.source();
        let binding = BoundSource::open(&source, None).unwrap();
        let mut holder = Some(binding.store.replace(binding.key.clone()).unwrap());
        let mut waited = false;
        run(&source, None, &AtomicBool::new(false), |accepted| {
            if let Some(row) = accepted.sections.iter().find(|row| row.id == "writer") {
                assert_eq!(row.label, "Waiting for another writer");
                assert_eq!(row.state, State::Preparing);
                assert!(row.error.is_none());
                waited = true;
                drop(holder.take());
            }
        })
        .unwrap();
        assert!(waited);
        assert!(state(&source, None).forward_complete(replay.index.len()));
    }

    #[test]
    fn a_sharing_violation_on_the_manifest_fails_without_waiting() {
        let (replay, _) = scenario();
        let fixture = Fixture::new(&replay);
        let source = fixture.source();
        let published = ordinary(&source, None);
        let binding = BoundSource::open(&source, None).unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(binding.store.directory().join("manifest"))
            .unwrap();
        let cancelled = AtomicBool::new(false);
        let mut waited = false;
        let error = super::super::wait_for_writer(
            || binding.store.extend(&binding.key),
            &cancelled,
            &published,
            &mut |_| {
                waited = true;
                cancelled.store(true, Ordering::Release);
            },
        )
        .err()
        .unwrap();
        assert!(!waited);
        assert!(!store::is_writer_busy(&error));
        assert_eq!(
            error.raw_os_error(),
            Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32)
        );
        drop(held);
    }

    #[test]
    fn a_recording_stamp_change_without_a_manifest_change_requires_reopen_without_replanning() {
        let (replay, blocks) = scenario();
        let fixture = Fixture::new(&replay);
        let source = fixture.source();
        let mut family = begin(&source, None);
        feed(&mut family, &blocks, 1);
        assert_eq!(family.publish_saved().unwrap(), Publication::Published);
        let manifest = std::fs::read(fixture.store.directory().join("manifest")).unwrap();
        let mut holder = Some(fixture.store.extend(&family.binding.key).unwrap());
        let mut changed = false;
        let error = without_replanning(|| {
            run(&source, None, &AtomicBool::new(false), |accepted| {
                if accepted.sections.iter().any(|row| row.id == "writer") {
                    assert!(!changed);
                    let file = std::fs::OpenOptions::new()
                        .write(true)
                        .open(&fixture.path)
                        .unwrap();
                    let modified = file.metadata().unwrap().modified().unwrap();
                    file.set_times(
                        std::fs::FileTimes::new().set_modified(modified + Duration::from_secs(5)),
                    )
                    .unwrap();
                    changed = true;
                    drop(holder.take());
                }
            })
            .unwrap_err()
        });
        assert!(changed);
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            "Recording changed; reopen it before preparing the current content"
        );
        assert_eq!(
            std::fs::read(fixture.store.directory().join("manifest")).unwrap(),
            manifest
        );
    }

    #[test]
    fn cancellation_during_a_writer_wait_ends_before_another_acquisition() {
        let (replay, _) = scenario();
        let fixture = Fixture::new(&replay);
        let source = fixture.source();
        let binding = BoundSource::open(&source, None).unwrap();
        let _holder = binding.store.replace(binding.key.clone()).unwrap();
        let cancelled = AtomicBool::new(false);
        let mut waited = false;
        let error = run(&source, None, &cancelled, |accepted| {
            if accepted.sections.iter().any(|row| row.id == "writer") {
                assert!(!waited);
                waited = true;
                cancelled.store(true, Ordering::Release);
            }
        })
        .unwrap_err();
        assert!(waited);
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(payloads(&source, None).is_empty());
    }

    #[test]
    fn runner_shutdown_cancels_an_active_writer_wait_without_releasing_the_other_holder() {
        let (replay, _) = scenario();
        let fixture = Fixture::new(&replay);
        let source = fixture.source();
        let binding = BoundSource::open(&source, None).unwrap();
        let _holder = binding.store.replace(binding.key.clone()).unwrap();
        let runner = crate::prepared::runner::Runner::new().unwrap();
        drop(binding);
        let (waiting, observed) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        runner
            .submit(Arc::new(AtomicBool::new(false)), move |cancelled| {
                let result = run(&source, None, cancelled, |accepted| {
                    if accepted.sections.iter().any(|row| row.id == "writer") {
                        waiting.send(()).unwrap();
                    }
                });
                done.send(result.unwrap_err().kind()).unwrap();
            })
            .unwrap();
        observed.recv_timeout(Duration::from_secs(10)).unwrap();
        runner.shutdown();
        assert_eq!(
            finished.recv_timeout(Duration::from_secs(2)).unwrap(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn app_replans_if_external_publication_happens_between_its_load_and_writer_acquisition() {
        let (replay, blocks) = scenario();
        for mode in [
            "empty",
            "same binding",
            "different binding",
            "grown binding",
        ] {
            let existing = mode == "same binding";
            let first =
                Replay::from_records(CAMPAIGN.into(), &[records(1, blocks[0].clone())]).unwrap();
            let fixture = Fixture::new(if mode == "grown binding" {
                &first
            } else {
                &replay
            });
            if mode == "grown binding" {
                let prior = fixture.source();
                let mut family = begin(&prior, None);
                feed(&mut family, &blocks[..1], 1);
                assert_eq!(family.publish_saved().unwrap(), Publication::Published);
                drop(family);
                fixture.replace(&replay);
            }
            if mode == "different binding" {
                let input = fixture.root.join("synthetic-mod-input");
                std::fs::write(&input, "synthetic prior binding").unwrap();
                let old = store::SourceKey::capture(&fixture.path, &[input]).unwrap();
                fixture.store.replace(old).unwrap().commit().unwrap();
            }
            let source = fixture.source();
            let recorder = fixture.source();
            let mut family = begin(&recorder, None);
            if existing {
                feed(&mut family, &blocks, 1);
                assert_eq!(family.publish_saved().unwrap(), Publication::Published);
            }
            feed(&mut family, &blocks, blocks.len());
            let binding = BoundSource::open(&source, None).unwrap();
            let mut holder = Some(
                if existing {
                    binding.store.extend(&binding.key)
                } else {
                    binding.store.replace(binding.key.clone())
                }
                .unwrap(),
            );
            let mut published = false;
            let extracted = Arc::new(Mutex::new(Vec::new()));
            let seen = Arc::clone(&extracted);
            let hook: prepass::Hook = Arc::new(move |name, frame| {
                if name == AirStatisticsObserver::NAME {
                    seen.lock().unwrap().push(frame.hour);
                }
                prepass::Fault::Continue
            });
            prepass::with_test_prepass(1, Some(hook), || {
                run(&source, None, &AtomicBool::new(false), |accepted| {
                    if accepted.sections.iter().any(|row| row.id == "writer") && !published {
                        drop(holder.take());
                        assert_eq!(family.publish_saved().unwrap(), Publication::Published);
                        published = true;
                    }
                })
            })
            .unwrap();
            assert!(published);
            let binding = BoundSource::open(&source, None).unwrap();
            assert!(binding.store.is_reusable(&binding.key).unwrap());
            let (_, cached) = super::super::super::files(&source, None).unwrap();
            assert!(fixture.store.is_reusable(&cached).unwrap());
            assert!(
                extracted.lock().unwrap().is_empty(),
                "the stale plan must observe no air-statistics block"
            );
            let finished = payloads(&source, None);
            drop(family);
            fixture.store.clear().unwrap();
            let other = fixture.source();
            ordinary(&other, None);
            assert_eq!(finished, payloads(&other, None), "{mode}");
        }
    }

    #[test]
    fn a_changed_recorded_mod_input_refuses_the_checkpoint_and_prepares_the_family_normally() {
        let (replay, blocks) = scenario();
        let fixture = Fixture::new(&replay);
        let game = fixture.root.join("game");
        let addon = fixture.root.join("addon");
        for (root, file, text) in [
            (&game, "hoi4.exe", "test".to_owned()),
            (&game, "user/settings.txt", "language=l_english".to_owned()),
            (
                &game,
                "user/dlc_load.json",
                r#"{"enabled_mods":["mod/test.mod"],"disabled_dlcs":[]}"#.to_owned(),
            ),
            (
                &game,
                "user/mod/test.mod",
                format!(
                    r#"name="Synthetic recorded mod" path="{}""#,
                    addon.to_string_lossy().replace('\\', "/")
                ),
            ),
            (
                &addon,
                "common/defines/test.lua",
                "NDefines.NGame.SYNTHETIC=1".to_owned(),
            ),
        ] {
            let file = root.join(file);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        let (options, identity) = crate::test_fixtures::content_identity(&game);
        let mut input = records(1, blocks[0].clone());
        let Record::SessionStart(session) = &mut input[0] else {
            unreachable!()
        };
        session.identity = identity;
        let recording = Replay::from_records(CAMPAIGN.into(), &[input]).unwrap();
        fixture.replace(&recording);
        let source = fixture.source();
        let mut family = begin(&source, Some(&options));
        feed(&mut family, &blocks[..1], 1);
        assert_eq!(family.publish_saved().unwrap(), Publication::Published);
        let old_key = family.binding.key.clone();
        drop(family);
        ordinary(&source, Some(&options));
        let rejected = fixture.source();
        let stale = BoundSource::open(&rejected, Some(&options)).unwrap();
        let writer = stale.store.extend(&stale.key).unwrap();
        let mod_input = addon.join("common/defines/test.lua");
        let modified = std::fs::metadata(&mod_input).unwrap().modified().unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&mod_input)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified + Duration::from_secs(5)))
            .unwrap();
        assert!(writer.commit().is_err());
        assert_eq!(stale.store.saved_key().unwrap().as_ref(), Some(&stale.key));
        assert!(!stale.store.is_reusable(&stale.key).unwrap());
        let error = without_replanning(|| {
            run(&rejected, Some(&options), &AtomicBool::new(false), |_| {}).unwrap_err()
        });
        assert!(error.to_string().contains("reopen"));
        assert!(!stale.store.is_reusable(&stale.key).unwrap());
        let reopened = fixture.source();
        let binding = BoundSource::open(&reopened, Some(&options)).unwrap();
        assert_ne!(binding.key, old_key);
        assert!(
            binding
                .checkpoint(binding.saved().unwrap().as_ref())
                .unwrap()
                .is_none()
        );
        let family = ExternalFamily::begin(binding, Family::AirStatistics, 1).unwrap();
        assert_eq!(family.pending.as_ref().unwrap().progress.next_block, 0);
        drop(family);
        let observed = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&observed);
        let hook: prepass::Hook = Arc::new(move |name, _| {
            if name == AirStatisticsObserver::NAME {
                seen.store(true, Ordering::Release);
            }
            prepass::Fault::Continue
        });
        prepass::with_test_prepass(1, Some(hook), || {
            run(&reopened, Some(&options), &AtomicBool::new(false), |_| {})
        })
        .unwrap();
        assert!(observed.load(Ordering::Acquire));
        let prepared = air(&payloads(&reopened, Some(&options)));
        fixture.store.clear().unwrap();
        let fresh = fixture.source();
        ordinary(&fresh, Some(&options));
        assert_eq!(air(&payloads(&fresh, Some(&options))), prepared);
        std::fs::write(&mod_input, "NDefines.NGame.SYNTHETIC=200").unwrap();
        let mismatched = fixture.source();
        assert!(BoundSource::open(&mismatched, Some(&options)).is_err());
        let (_, identity) = crate::test_fixtures::content_identity(&game);
        let mut input = records(1, blocks[0].clone());
        let Record::SessionStart(session) = &mut input[0] else {
            unreachable!()
        };
        session.identity = identity;
        let changed = Replay::from_records(CAMPAIGN.into(), &[input]).unwrap();
        fixture.replace(&changed);
        let rebound = fixture.source();
        let binding = BoundSource::open(&rebound, Some(&options)).unwrap();
        assert!(
            binding
                .checkpoint(binding.saved().unwrap().as_ref())
                .unwrap()
                .is_none()
        );
        ordinary(&rebound, Some(&options));
        let prepared = payloads(&rebound, Some(&options));
        fixture.store.clear().unwrap();
        let fresh = fixture.source();
        ordinary(&fresh, Some(&options));
        assert_eq!(payloads(&fresh, Some(&options)), prepared);
    }
}
