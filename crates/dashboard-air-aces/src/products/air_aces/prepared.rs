//! Block roster replies, sparse naming observations and compact portrait selections.
use super::*;
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use dashboard_text::metadata::check_cancel;
use replay_data::{codec::Decoder, rows::TableRows};

const ROSTER_READ_HOURS: i32 = 24;

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
    /// The variant catalogue's keys, which a continuation saves beside it.
    pub(super) fn keys(&self) -> &BTreeMap<String, u64> {
        &self.keys
    }
    pub(super) fn set_keys(&mut self, keys: BTreeMap<String, u64>) {
        self.keys = keys;
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
        let (section, window, reply) =
            self.window_at_end(source, replay, block, context, cancelled)?;
        series::write_reply(writer, &section, window, &reply)?;
        self.forget_closed_runs();
        Ok(())
    }
    /// The window reply of `block`, at its end: its section and window.
    fn window_at_end(
        &mut self,
        source: &Source,
        replay: &Replay,
        block: usize,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<(String, u64, AirAcesWindowReply)> {
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
        Ok((
            format!("air-aces-{}-windows", index.branch),
            window as u64,
            reply,
        ))
    }
    /// After a block's window is published: completed runs live only in the
    /// window files. All ace identities keep their bounds.
    fn forget_closed_runs(&mut self) {
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
        writer.write_at(&format!("air-aces-{branch}"), 0, bytes.as_slice())?;
        #[cfg(any(test, feature = "test-support"))]
        dashboard_base::prepared::pages::framing::note_compressed(&bytes);
        Ok(())
    }
    pub(super) fn read(
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
    /// Every retained (ace, run): layer n holds each ace's nth run, ids ascending for the codec.
    fn run_layers(&self) -> Vec<Vec<(usize, usize)>> {
        let mut order: Vec<usize> = (0..self.scan.aces.len())
            .filter(|&index| !self.scan.aces[index].runs.is_empty())
            .collect();
        order.sort_unstable_by_key(|&index| self.scan.aces[index].id);
        let mut layers: Vec<Vec<(usize, usize)>> = Vec::new();
        for index in order {
            for run in 0..self.scan.aces[index].runs.len() {
                if layers.len() == run {
                    layers.push(Vec::new());
                }
                layers[run].push((index, run));
            }
        }
        layers
    }
    /// Each layer's count and rows, which the continuation does not hold ([`Self::run_layers`]).
    pub(super) fn run_rows(&self) -> io::Result<Vec<(usize, Vec<u8>)>> {
        self.run_layers()
            .into_iter()
            .map(|layer| {
                let rows = layer
                    .iter()
                    .map(|&(index, run)| (*self.scan.aces[index].runs[run].facts.row).clone())
                    .collect::<Vec<_>>();
                let mut bytes = Vec::new();
                TableRows::AirAces(rows).encode_rows(&mut bytes)?;
                Ok((layer.len(), bytes))
            })
            .collect()
    }
    /// Points every retained run at its row in `saved` ([`Self::run_rows`]).
    pub(super) fn restore_run_rows(&mut self, saved: &[(usize, Vec<u8>)]) -> io::Result<()> {
        let layers = self.run_layers();
        if layers.len() != saved.len() {
            return Err(dashboard_core::invalid("Ace continuation runs miss their rows"));
        }
        for (layer, (count, bytes)) in layers.into_iter().zip(saved) {
            if layer.len() != *count {
                return Err(dashboard_core::invalid("Ace continuation runs miss their rows"));
            }
            let mut input = Decoder::new(bytes);
            let rows = TableId::AirAces.decode_rows(*count, &mut input)?;
            input.finish()?;
            let ids = rows.rows::<tables::AirAces>().expect("decoded ace rows");
            if layer
                .iter()
                .zip(ids)
                .any(|(&(index, _), row)| row.id != self.scan.aces[index].id)
            {
                return Err(dashboard_core::invalid("Ace continuation row is another ace's"));
            }
            let table = SharedView::from(TableView::Complete(rows));
            for (position, (index, run)) in layer.into_iter().enumerate() {
                self.scan.aces[index].runs[run].facts.row = RecordedAce::Initial {
                    table: table.clone(),
                    index: position,
                };
            }
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn manifest(
        &mut self,
        source: &Source,
        store: &store::Store,
        key: &store::SourceKey,
        params: &AirAcesParams,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<AirAcesReply> {
        let cat = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options));
        let manifest =
            self.named_manifest(source, store, key, params, cat.as_deref().ok(), cancelled)?;
        with_warnings(manifest, cat.as_deref())
    }
    /// The manifest with its variants named through `cat`, before the
    /// content's warnings join it.
    fn named_manifest(
        &mut self,
        source: &Source,
        store: &store::Store,
        key: &store::SourceKey,
        params: &AirAcesParams,
        cat: Option<&Catalogue>,
        cancelled: &AtomicBool,
    ) -> io::Result<AirAcesManifestReply> {
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
        manifest(&self.scan, params, cat, cancelled)
    }
    #[cfg(test)]
    pub(super) fn portrait_at(&self, id: u64, hour: i32) -> Option<PortraitOperands> {
        portrait_at(
            &self.portraits,
            self.portrait_through,
            &self.portrait_active,
            id,
            hour,
        )
    }
}

fn portrait_at(
    portraits: &BTreeMap<u64, Vec<PortraitRun>>,
    through: Option<i32>,
    active: &BTreeSet<u64>,
    id: u64,
    hour: i32,
) -> Option<PortraitOperands> {
    let runs = portraits.get(&id)?;
    let index = runs
        .partition_point(|run| run.first <= hour)
        .checked_sub(1)?;
    let run = &runs[index];
    let last = if index + 1 == runs.len() && active.contains(&id) {
        through.unwrap_or(run.last)
    } else {
        run.last
    };
    (hour <= last).then(|| run.operands.clone())
}

/// The continuation's portrait runs, which are all a portrait reply reads.
type Portraits = (BTreeMap<u64, Vec<PortraitRun>>, Option<i32>, BTreeSet<u64>);

/// [`Portraits`] of a complete branch, kept for the source while its
/// continuation is unchanged.
fn kept_portraits(
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Portraits>> {
    let page = store
        .read(key, &format!("air-aces-{branch}"), 0)?
        .map(|file| file.id().cache_key());
    dashboard_base::prepared::query::observations::scope(source, "air-aces:portraits", Some(key))
        .get_entry(
            branch,
            &(),
            &page,
            cancelled,
            dashboard_base::prepared::query::observations::serde_bytes,
            || {
                let state = Prepared::read(store, key, None, branch)?;
                Ok((
                    state.portraits,
                    state.portrait_through,
                    state.portrait_active,
                ))
            },
        )
}

/// `manifest` with the content's warnings as they are now, within the reply bound.
fn with_warnings(
    manifest: AirAcesManifestReply,
    cat: Result<&Catalogue, &io::Error>,
) -> io::Result<AirAcesReply> {
    let mut reply = AirAcesReply::Manifest(manifest);
    ace_warnings(&mut reply, cat);
    bounded(reply, "manifest")
}

/// The branch's manifest, which does not depend on the hour asked: its named
/// variants are kept for the source while the pages they read and the
/// catalogue naming them are unchanged, and each reply takes the catalogue's
/// warnings as they are then.
fn kept_manifest(
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    params: &AirAcesParams,
    cat: &io::Result<Arc<Catalogue>>,
    cancelled: &AtomicBool,
) -> io::Result<AirAcesReply> {
    let branch = params.branch_id.parse::<u64>().expect("validated branch");
    // The catalogue's address names it: the kept weak reference keeps the
    // address from naming another catalogue while the manifest is kept.
    let pages = (
        store
            .read(key, &format!("air-aces-{branch}"), 0)?
            .map(|file| file.id().cache_key()),
        series::catalogue_pages(store, key, "air-aces", branch)?,
        Reader::new(store, key, format!("air-aces-{branch}-series"), cancelled)?.pages()?,
        cat.as_ref().ok().map(|cat| Arc::as_ptr(cat) as usize),
    );
    let kept = dashboard_base::prepared::query::observations::scope(
        source,
        "air-aces:manifest",
        Some(key),
    )
    .get_entry(
        branch,
        &params.branch_id,
        &pages,
        cancelled,
        |(_, manifest): &(Option<std::sync::Weak<Catalogue>>, AirAcesManifestReply)| {
            dashboard_base::prepared::query::observations::serde_bytes(manifest)
        },
        || {
            let mut state = Prepared::read(store, key, None, branch)?;
            let manifest =
                state.named_manifest(source, store, key, params, cat.as_deref().ok(), cancelled)?;
            Ok((cat.as_ref().ok().map(Arc::downgrade), manifest))
        },
    )?;
    with_warnings(kept.1.clone(), cat.as_deref())
}

pub(super) fn sample_roster(
    source: &Source,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<(Scan, Option<i32>)> {
    let replay = source.replay_ref();
    let declaration = replay
        .branches
        .iter()
        .find(|row| row.id == branch)
        .ok_or_else(|| dashboard_core::invalid("Branch not in this recording"))?;
    let (first, last) = branch_bounds(replay, declaration);
    let mut selected = vec![
        TableId::Campaign,
        TableId::AirAces,
        TableId::AirWings,
        TableId::WingEquipment,
        TableId::Raids,
        TableId::RaidTooltipOperands,
    ];
    selected.extend(VariantCatalogue::TABLES);
    selected.sort_unstable();
    selected.dedup();
    let frame = dashboard_base::prepared::query::progressive::latest_read(
        source,
        branch,
        hour,
        TableId::AirAces,
        ROSTER_READ_HOURS,
        &selected,
        cancelled,
    )?;
    let read_hour = frame.as_ref().map(|frame| frame.hour);
    let mut scan = Scan {
        format: replay.format,
        first,
        last,
        windows: vec![HourSpan {
            first_hour: read_hour.unwrap_or(hour),
            last_hour: hour,
        }],
        ..Default::default()
    };
    if let Some(frame) = frame {
        if let Some(variants) = dashboard_base::prepared::query::progressive::latest_read(
            source,
            branch,
            frame.hour,
            TableId::VariantDefinitions,
            replay_data::cadence::STAGGER_PERIOD,
            &VariantCatalogue::TABLES,
            cancelled,
        )? {
            scan.variants.observe(replay.format, &variants);
            scan.variants.advance(frame.hour);
        }
        scan.observe_roster(&frame, None);
        scan.close(hour);
        if let Some(observed) = scan.observed.last_mut() {
            observed.last_hour = hour;
        }
    }
    Ok((scan, read_hour))
}

fn roster(
    source: &Source,
    params: AirAcesRosterParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let branch = params
        .branch_id
        .parse::<u64>()
        .map_err(|_| dashboard_core::invalid("Invalid ace branch"))?;
    let prefix = dashboard_base::prepared::prefix::read(
        source,
        context.content.as_ref(),
        AirAces::KEY,
        branch,
        Some(params.hour),
        cancelled,
    )?;
    let replay = source.replay_ref();
    let declaration = replay
        .branches
        .iter()
        .find(|row| row.id == branch)
        .ok_or_else(|| dashboard_core::invalid("Branch not in this recording"))?;
    let (first, last) = branch_bounds(replay, declaration);
    if params.hour < first || params.hour > last {
        return Err(dashboard_core::invalid("Ace roster hour outside recording"));
    }
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let window = source
        .block_windows(branch)
        .partition_point(|window| window.last_hour < params.hour);
    let request = AirAcesParams {
        branch_id: params.branch_id.clone(),
        view: AirAcesView::Window,
        window: Some(window),
    };
    let mut reply = if prefix
        .as_ref()
        .is_some_and(|prefix| params.hour <= prefix.coverage.last_hour)
    {
        let cat = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options));
        let AirAcesReply::Manifest(manifest) =
            kept_manifest(source, &store, &key, &request, &cat, cancelled)?
        else {
            unreachable!()
        };
        let roster = series::read_reply::<AirAcesWindowReply>(
            &store,
            &key,
            &format!("air-aces-{branch}-windows"),
            window as u64,
        )?
        .ok_or_else(|| dashboard_core::invalid("Prepared ace window is missing"))?;
        let read_hour = manifest
            .read_hours
            .iter()
            .rev()
            .copied()
            .find(|&hour| hour <= params.hour);
        AirAcesRosterReply {
            manifest,
            roster,
            read_hour,
            preparing: vec![],
        }
    } else {
        let (scan, read_hour) = sample_roster(source, branch, params.hour, cancelled)?;
        let cat = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options));
        let sampled = AirAcesParams {
            window: Some(0),
            ..request
        };
        let manifest = super::manifest(&scan, &sampled, cat.as_deref().ok(), cancelled)?;
        let mut roster = window_reply(&scan, &sampled, cat.as_deref().ok(), cancelled)?;
        for ace in &mut roster.aces {
            ace.first_seen = None;
            for run in &mut ace.runs {
                if let Some(death) = &mut run.death {
                    death.last_alive = None;
                    death.by_hour = None;
                }
            }
        }
        AirAcesRosterReply {
            manifest,
            roster,
            read_hour,
            preparing: vec!["firstSeen".into(), "death".into()],
        }
    };
    // First sightings are not known from one roster read.
    if !reply.preparing.is_empty() {
        reply.manifest.aces.clear();
    }
    let mut data = serde_json::to_value(reply)?;
    match prefix {
        Some(prefix) => prefix.mark(&mut data)?,
        None => dashboard_base::prepared::prefix::mark_unprepared(
            source,
            branch,
            Some(params.hour),
            &mut data,
        )?,
    }
    dashboard_base::products::check_reply_budget(&data)?;
    Ok(data)
}

pub fn query(
    source: &Source,
    product: &str,
    params: serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    if product == AirAces::KEY
        && params.get("view").and_then(serde_json::Value::as_str) == Some("roster")
    {
        return roster(source, serde_json::from_value(params)?, context, cancelled);
    }
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
        let coverage = dashboard_base::prepared::coverage::product_coverage(
            source,
            context.content.as_ref(),
            branch,
            AirAcePortraits::KEY,
            cancelled,
        )?;
        if coverage
            .as_ref()
            .is_none_or(|coverage| !coverage.complete && params.hour > coverage.last_hour)
        {
            let (scan, _) = sample_roster(source, branch, params.hour, cancelled)?;
            return serde_json::to_value(portrait_reply(
                source,
                &params,
                context,
                cancelled,
                |id, _| {
                    scan.aces
                        .iter()
                        .find(|ace| ace.id == id)
                        .and_then(|ace| ace.runs.last())
                        .map(|run| PortraitOperands::from(&run.facts))
                },
            )?)
            .map_err(Into::into);
        }
        let portraits = kept_portraits(source, &store, &key, branch, cancelled)?;
        let (runs, through, active) = &*portraits;
        return serde_json::to_value(portrait_reply(
            source,
            &params,
            context,
            cancelled,
            |id, hour| portrait_at(runs, *through, active, id, hour),
        )?)
        .map_err(Into::into);
    }
    let params: AirAcesParams = serde_json::from_value(params)?;
    AirAces::validate(&params).map_err(dashboard_core::invalid)?;
    let branch = params.branch_id.parse().expect("validated branch");
    let Some(coverage) = dashboard_base::prepared::coverage::product_coverage(
        source,
        context.content.as_ref(),
        branch,
        AirAces::KEY,
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
    if !matches!(params.view, AirAcesView::Catalogue) {
        check_cancel(cancelled)?;
        let cat = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options));
        return serde_json::to_value(kept_manifest(
            source, &store, &key, &params, &cat, cancelled,
        )?)
        .map_err(Into::into);
    }
    let state = Prepared::read(&store, &key, None, branch)?;
    Ok(
        serde_json::json!({"available": !state.scan.observed.is_empty(),
        "unavailable": if state.scan.observed.is_empty() { Some(if state.scan.unread {
            "AirAces availability could not be checked"
        } else { "AirAces was unavailable, incomplete or never read in this recording." }) } else { None }}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::prepared::series::tests::Fixture;
    use dashboard_core::test_fixtures::{CAMPAIGN, records, scale_frame};
    use replay_data::rows::TableRows;

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

    fn intervening_country_identity(count: u32) {
        use dashboard_base::prepared::frame_changes::{FrameChangeTracker, FrameChanges};
        use replay_data::rows::CountryRow;
        use replay_frame::{frame::FrameDecoder, frame::FrameEncoder};
        for invalidation in [
            "rename",
            "rename_without_delta",
            "link",
            "slot",
            "delete",
            "insert",
            "snapshot",
            "gap",
            "unavailable",
        ] {
            let mut input =
                super::super::tests::frame(117, vec![super::super::tests::ace(1, true)]);
            let mut countries = input.complete::<tables::Countries>().unwrap().to_vec();
            countries[0].tag = b"AAA".to_vec();
            countries.extend((1..count).map(|id| CountryRow {
                id: 9001 + id,
                canonical_slot: 7 + id,
                tag: format!("mod country {id}").into_bytes(),
                ..Default::default()
            }));
            countries[1].tag = b"BBB".to_vec();
            *input.tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(countries.clone()));
            let replay = dashboard_core::test_fixtures::replay(vec![input.clone()]);
            let mut encoder = FrameEncoder::new();
            let mut decoder = FrameDecoder::new();
            let mut tracker = FrameChangeTracker::new();
            let mut state = Prepared::default();
            let mut oracle = Prepared::default();
            for hour in 117..=141 {
                if invalidation == "gap" && hour == 118 {
                    continue;
                }
                input.hour = hour;
                input.metrics.table_ns.fill(0);
                if hour == 117 || hour == 141 {
                    input.metrics.table_ns[TableId::AirAces.index()] = 1;
                }
                if hour == 118 {
                    match invalidation {
                        "rename" | "rename_without_delta" => countries[0].tag = b"BBB".to_vec(),
                        "link" => {
                            if let TableView::Complete(TableRows::CountryLinks(rows)) =
                                &mut *input.tables[TableId::CountryLinks.index()]
                            {
                                rows.iter_mut()
                                    .find(|row| row.id == 11)
                                    .unwrap()
                                    .canonical_slot = countries[1].canonical_slot;
                            }
                        }
                        "slot" => countries[0].canonical_slot += 100_000,
                        "delete" => {
                            countries.remove(0);
                        }
                        "insert" => countries.push(CountryRow {
                            id: 100_000,
                            canonical_slot: 100_000,
                            tag: b"mod added".to_vec(),
                            ..Default::default()
                        }),
                        _ => {}
                    }
                }
                if hour != 141 || invalidation != "rename_without_delta" {
                    countries[0].political_power_raw = Some(i64::from(hour));
                }
                *input.tables[TableId::Countries.index()] =
                    if hour == 118 && invalidation == "unavailable" {
                        TableView::Unavailable {
                            reason: "fixture country refusal".into(),
                        }
                    } else {
                        TableView::Complete(TableRows::Countries(countries.clone()))
                    };
                let wire = Arc::new(
                    encoder
                        .encode(
                            input.clone(),
                            hour == 117 || hour == 118 && invalidation == "snapshot",
                        )
                        .unwrap(),
                );
                let frame = decoder.advance(&wire).unwrap();
                let changes = tracker.observe_updates(frame, wire.clone(), replay.format);
                let before = state.scan.work.row_visits;
                let keys = state.scan.country_keys.clone();
                let key_visits = keys.as_ref().map_or(0, |keys| keys.row_visits());
                state.observe_changed(frame, &replay, &changes).unwrap();
                if hour == 141 && matches!(invalidation, "snapshot" | "unavailable") {
                    assert_eq!(
                        keys.unwrap().row_visits() - key_visits,
                        count as usize,
                        "{invalidation}: the next roster must compare the full country projection"
                    );
                }
                oracle
                    .observe_changed(frame, &replay, &FrameChanges::alone(frame, replay.format))
                    .unwrap();
                if hour != 117 && hour != 141 {
                    assert!(
                        state.scan.work.row_visits - before <= 1,
                        "{invalidation}: unread hour must not walk {count} country keys"
                    );
                }
            }
            state.scan.close(141);
            oracle.scan.close(141);
            state.seal_portraits();
            oracle.seal_portraits();
            assert_eq!(
                serde_json::to_vec(&state).unwrap(),
                serde_json::to_vec(&oracle).unwrap(),
                "{invalidation}: continuation and portrait bytes"
            );
            if invalidation.starts_with("rename") || invalidation == "link" {
                assert_eq!(
                    state.scan.aces[0].runs.last().unwrap().facts.tag.as_deref(),
                    Some("BBB")
                );
                assert_eq!(
                    state.portrait_at(1, 141).unwrap().tag.as_deref(),
                    Some("BBB")
                );
            }
        }
    }

    #[test]
    fn aces_country_identity_changes_between_roster_reads_match_hourly_bytes() {
        intervening_country_identity(8);
    }

    #[test]
    fn aces_shared_driver_equal_reads_defer_roster_and_portrait_dates() {
        use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let measured = observer_tests::scaled_work(8, |hours| {
            let fixture = Fixture::new();
            let flag = AtomicBool::new(false);
            let mut first = scale_frame(1, 10, 24);
            first.metrics.table_ns[TableId::AirAces.index()] = 1;
            first.tables[TableId::AirAces.index()] = TableView::Complete(TableRows::AirAces(
                (0..24)
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
    fn publish(state: &mut Prepared, writer: &mut store::Writer, source: &Source, block: usize) {
        let flag = AtomicBool::new(false);
        let updates = source
            .replay_ref()
            .read_block_selected(&source.replay_ref().index[block], &TableId::ALL)
            .unwrap();
        let mut decoder = replay_frame::frame::FrameDecoder::for_tables(&TableId::ALL);
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        for update in updates {
            let frame = decoder.advance(&update).unwrap();
            state
                .observe_changed(
                    frame,
                    source.replay_ref(),
                    &tracker.observe(frame, source.replay_ref().format),
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
    fn kept_manifests_take_each_requests_content_warnings() {
        let fixture = Fixture::new();
        let frames = vec![super::super::tests::frame(
            10,
            vec![super::super::tests::ace(1, true)],
        )];
        let recorded =
            source(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
        let mut state = Prepared::default();
        let mut writer = fixture.writer();
        publish(&mut state, &mut writer, &recorded, 0);
        writer.commit().unwrap();
        let (cat, _folder) = catalogue::tests::folder_catalogue(&[]);
        let cat: io::Result<Arc<Catalogue>> = Ok(Arc::new(cat));
        let params = AirAcesParams {
            branch_id: "1".into(),
            view: AirAcesView::Manifest,
            window: None,
        };
        let flag = AtomicBool::new(false);
        let read = || {
            serde_json::to_value(
                kept_manifest(
                    &recorded,
                    &fixture.store,
                    &fixture.key,
                    &params,
                    &cat,
                    &flag,
                )
                .unwrap(),
            )
            .unwrap()
        };
        let refused = |reply: &serde_json::Value| {
            reply["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|line| line.as_str().unwrap().contains("synthetic refusal"))
        };
        let before = read();
        assert!(!refused(&before));
        // Another product's refusal joins the shared catalogue between two reads.
        cat.as_ref().unwrap().refuse("synthetic refusal", "another text");
        let after = read();
        assert!(refused(&after));
        let mut fresh = Prepared::read(&fixture.store, &fixture.key, None, 1).unwrap();
        let expected = with_warnings(
            fresh
                .named_manifest(
                    &recorded,
                    &fixture.store,
                    &fixture.key,
                    &params,
                    cat.as_deref().ok(),
                    &flag,
                )
                .unwrap(),
            cat.as_deref(),
        )
        .unwrap();
        assert_eq!(after, serde_json::to_value(expected).unwrap());
        assert_eq!(
            dashboard_base::prepared::query::observations::usage(&recorded, "air-aces:manifest").2,
            1
        );
    }

    #[test]
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
