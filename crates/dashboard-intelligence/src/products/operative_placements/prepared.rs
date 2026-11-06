//! Placement changes are append-only events; continuation retains the latest
//! pair values. Successful membership dates use separate shared series runs.
use super::*;
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    series::{self, Accumulator, Reader},
};
use dashboard_text::metadata::check_cancel;

const COVERAGE: u64 = 0;
const MEMBERSHIP_READ: u64 = 1;

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    values: Accumulator,
    keys: BTreeMap<String, u64>,
    first: Option<i32>,
    last: Option<i32>,
    active: std::collections::BTreeSet<PairKey>,
    membership_dirty: bool,
    /// Every pair has been told since this state was made or read back.
    #[serde(skip)]
    bound: bool,
    /// The pairs the reads handed on changed since the read the history last took, or all.
    #[serde(skip)]
    pending: std::collections::BTreeSet<PairKey>,
    #[serde(skip)]
    pending_all: bool,
    /// The row path's last read ([`Self::observe_bound`]).
    #[serde(skip)]
    held: Option<PlacementRead>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    test_countries: super::super::operative_rows::CountryInputs,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Params {
    pub branch_id: String,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
}

impl Prepared {
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(
        &mut self,
        source: &dashboard_base::Source,
        context: &dashboard_core::config::LoadContext,
        frame: &DecodedFrame,
    ) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, source.replay_ref().format);
        let bound = context
            .content
            .as_ref()
            .map(|options| dashboard_base::catalogue::catalogue(source, options))
            .transpose()
            .map_err(dashboard_text::catalogue::ContentError::from);
        self.test_countries.update(frame, Some(&changes));
        let countries = std::mem::take(&mut self.test_countries);
        let result = self.observe_bound(frame, &changes, &countries, &bound);
        self.test_countries = countries;
        result
    }
    /// Observes `frame` of the row path, its census read where the scan reads it.
    pub fn observe_bound(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
        content: &Result<
            Option<std::sync::Arc<dashboard_text::catalogue::Catalogue>>,
            dashboard_text::catalogue::ContentError,
        >,
    ) -> io::Result<()> {
        let census = Census::of(frame);
        let mut touched = PlacementTouched::default();
        if self.reads(frame.hour, census, changes, countries) {
            let captured = recorded_operatives(frame).expect("a complete census");
            self.scan.load(content, captured.is_empty())?;
            let read = read(
                captured,
                countries,
                self.scan
                    .catalogue
                    .as_ref()
                    .map(|catalogue| catalogue.localisation.as_ref()),
            );
            if let Some(delta) = read.delta(self.held.as_ref()) {
                PlacementRead::apply(&mut self.held, delta, &mut touched);
            }
        }
        let held = self.held.take();
        let result = self.observe_read(
            frame.hour,
            census,
            changes,
            countries,
            content,
            held.as_ref(),
            &touched,
        );
        self.held = held;
        result
    }

    /// Whether `hour` (whose census is `census`) is observed from the census read.
    fn reads(
        &self,
        hour: i32,
        census: Census,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
    ) -> bool {
        let (changed, needs_read) = self.changes(hour, census, changes, countries.changed);
        (changed || needs_read) && census.resolves(countries)
    }

    /// Whether `hour` changed what the scan observes, and whether it needs the census read.
    fn changes(
        &self,
        hour: i32,
        census: Census,
        changes: &FrameChanges,
        countries_changed: bool,
    ) -> (bool, bool) {
        let changed = countries_changed
            || self.last.is_none_or(|last| last.checked_add(1) != Some(hour))
            || [TableId::SimulationFeatures, TableId::OperativeMemberships]
                .iter()
                .any(|table| changes.rows_changed[table.index()]);
        let dirty = self.membership_dirty
            || countries_changed
            || changes.rows_changed[TableId::OperativeMemberships.index()];
        let needs_read = census.read
            && (dirty || self.scan.history.held.is_none())
            && census.feature == Some(true);
        (changed, needs_read)
    }

    /// Observes `hour`, whose change facts are `changes`, from the census `read` held, of which
    /// `touched` names the pairs the hour changed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_read(
        &mut self,
        hour: i32,
        census: Census,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
        content: &Result<
            Option<std::sync::Arc<dashboard_text::catalogue::Catalogue>>,
            dashboard_text::catalogue::ContentError,
        >,
        read: Option<&PlacementRead>,
        touched: &PlacementTouched,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("intelligence:placements");
        let is_read = census.read;
        let (changed, needs_read) = self.changes(hour, census, changes, countries.changed);
        self.membership_dirty |=
            countries.changed || changes.rows_changed[TableId::OperativeMemberships.index()];
        self.values.advance(hour)?;
        self.pending_all |= touched.all || !self.bound;
        self.pending.extend(touched.pairs.iter().cloned());
        // The pairs told; None for every pair held.
        let mut told = Some(Vec::new());
        if changed || needs_read {
            let patch: Option<Vec<PairKey>> =
                (!self.pending_all).then(|| self.pending.iter().cloned().collect());
            let observed = self.scan.observe_read(
                hour,
                census,
                countries,
                content,
                read,
                patch.as_deref(),
                countries.changed || !self.bound,
            )?;
            self.bound = true;
            // A read the history took leaves nothing pending.
            if matches!(observed, Told::Pairs(_) | Told::Read) {
                self.pending.clear();
                self.pending_all = false;
            }
            told = match observed {
                Told::Pairs(keys) => Some(keys),
                Told::Held => Some(Vec::new()),
                Told::Read | Told::All => None,
            };
        } else {
            self.scan.history.last_hour = Some(hour);
            self.scan
                .history
                .periods
                .last_mut()
                .expect("placement period")
                .last_hour = hour;
            if is_read && self.scan.history.held.is_some() {
                self.scan.history.reads.push(hour);
            }
        }
        if is_read {
            self.membership_dirty = false;
        }
        self.first.get_or_insert(hour);
        self.last = Some(hour);
        let history = &mut self.scan.history;
        let period = history.periods.last().expect("observed placement coverage");
        let starts = period.first_hour == hour;
        if starts {
            self.values.observe(
                COVERAGE,
                hour,
                Some(i128::from(period.first_hour)),
                true,
            );
            self.values.event(COVERAGE, hour, period)?;
        }
        let read = history
            .reads
            .last()
            .copied()
            .filter(|_| period.availability == PlacementAvailability::Recorded);
        if starts || read == Some(hour) {
            self.values
                .observe(MEMBERSHIP_READ, hour, read.map(i128::from), starts);
        }
        if read == Some(hour) {
            self.values
                .event(MEMBERSHIP_READ, hour, &hour)?;
        }
        if changed || needs_read {
            let held = history.held.as_ref();
            let told = match told {
                Some(keys) => keys,
                None => {
                    let active: std::collections::BTreeSet<PairKey> = held
                        .into_iter()
                        .flat_map(|held| held.keys().cloned())
                        .collect();
                    let mut keys: Vec<PairKey> = self.active.difference(&active).cloned().collect();
                    keys.extend(active);
                    keys
                }
            };
            for pair_key in &told {
                if !held.is_some_and(|held| held.contains_key(pair_key)) {
                    if self.active.remove(pair_key) {
                        let pair = &history.pairs[pair_key];
                        self.values.observe(self.keys[&pair.id], hour, None, false);
                    }
                    continue;
                }
                let pair = history
                    .pairs
                    .get_mut(pair_key)
                    .expect("live placement pair");
                let key = match self.keys.get(&pair.id) {
                    Some(&key) => key,
                    None => {
                        let next = u64::try_from(self.keys.len())
                            .ok()
                            .and_then(|len| len.checked_add(2))
                            .ok_or_else(|| io::Error::other("Too many placement pairs"))?;
                        self.keys.insert(pair.id.clone(), next);
                        next
                    }
                };
                let run = pair.runs.last().expect("live placement run");
                if run.first_hour == hour || !self.active.contains(pair_key) {
                    self.values.observe(
                        key,
                        hour,
                        Some(i128::from(run.first_hour)),
                        run.first_hour == hour,
                    );
                }
                if run.first_hour == hour {
                    #[cfg(test)]
                    super::super::intelligence_prepared::work(|work| work.emitted += 1);
                    self.values.event(key, hour, run)?;
                }
                if pair.runs.len() > 1 {
                    pair.runs.drain(..pair.runs.len() - 1);
                }
                self.active.insert(pair_key.clone());
            }
        }
        if history.periods.len() > 1 {
            history.periods.drain(..history.periods.len() - 1);
        }
        if history.reads.len() > 1 {
            history.reads.drain(..history.reads.len() - 1);
        }
        Ok(())
    }

    pub fn bytes(&self) -> usize {
        self.values.bytes()
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if let Some(last) = self.last {
            for key in &self.active {
                if let Some(run) = self
                    .scan
                    .history
                    .pairs
                    .get_mut(key)
                    .and_then(|pair| pair.runs.last_mut())
                {
                    run.last_hour = last;
                }
            }
        }
        self.values.flush(
            writer,
            &format!("operative-placements-{branch}-series"),
            0,
            cancelled,
        )?;
        if let Some(bounds) = self.first.zip(self.last) {
            series::write_catalogue(writer, "operative-placements", branch, &self.keys, &bounds)?;
        }
        series::write_reply(writer, &format!("operative-placements-{branch}"), 0, self)
    }

    pub fn read(reader: impl io::Read) -> io::Result<Self> {
        Ok(serde_json::from_reader(zstd::stream::Decoder::new(
            reader,
        )?)?)
    }
}

pub fn query(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    params: &Params,
    cancelled: &AtomicBool,
) -> io::Result<Option<OperativePlacementReply>> {
    if params.branch_id.parse::<u64>().ok() != Some(branch) {
        return Err(dashboard_core::invalid(
            "Placement parameters do not match the dispatched branch",
        ));
    }
    let Some((keys, (first, last))) =
        series::read_catalogue::<(i32, i32)>(store, key, "operative-placements", branch)?
    else {
        return Ok(None);
    };
    let Some(state) =
        series::read_reply::<Prepared>(store, key, &format!("operative-placements-{branch}"), 0)?
    else {
        return Ok(None);
    };
    let mut first = params.first_hour.unwrap_or(first);
    let last = params.last_hour.unwrap_or(last);
    if first > last {
        return Err(dashboard_core::invalid("Invalid placement query range"));
    }
    let mut reader = Reader::new(
        store,
        key,
        format!("operative-placements-{branch}-series"),
        cancelled,
    )?;
    // A window includes the actual read supplying its left edge. This retains
    // held identity changes and satisfies the product's original coverage checks.
    if let Some(read) = reader.value_at(MEMBERSHIP_READ, first)? {
        first = i32::try_from(read.value)
            .map_err(|_| dashboard_core::invalid("Invalid membership date"))?;
    }
    let mut scan = Scan {
        warnings: state.scan.warnings,
        ..Default::default()
    };
    for span in reader.runs(COVERAGE, first, last)? {
        check_cancel(cancelled)?;
        let (_, row) = reader
            .latest_event(COVERAGE, span.first_hour)?
            .ok_or_else(|| dashboard_core::invalid("Missing placement coverage event"))?;
        let mut period: PlacementPeriod = serde_json::from_slice(&row)?;
        period.first_hour = span.first_hour;
        period.last_hour = span.last_hour;
        scan.history.periods.push(period);
    }
    scan.history.reads = reader
        .events(&[MEMBERSHIP_READ], first, last, 0, u32::MAX)?
        .rows
        .into_iter()
        .map(|row| serde_json::from_slice(&row.row).map_err(io::Error::from))
        .collect::<io::Result<_>>()?;
    for (id, key) in keys {
        check_cancel(cancelled)?;
        let (owner, target) = serde_json::from_str(&id)?;
        let pair_key = PairKey { owner, target };
        let mut pair = OperativePlacementPair {
            id,
            owner: pair_key.owner.2.clone(),
            target: pair_key.target.2.clone(),
            runs: Vec::new(),
        };
        for span in reader.runs(key, first, last)? {
            let (_, row) = reader
                .latest_event(key, span.first_hour)?
                .ok_or_else(|| dashboard_core::invalid("Missing placement value event"))?;
            let mut run: OperativePlacementRun = serde_json::from_slice(&row)?;
            run.first_hour = span.first_hour;
            run.last_hour = span.last_hour;
            pair.runs.push(run);
        }
        if !pair.runs.is_empty() {
            scan.history.pairs.insert(pair_key, pair);
        }
    }
    let reply = scan.reply(branch);
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(Some(reply))
}

#[cfg(test)]
mod tests;
