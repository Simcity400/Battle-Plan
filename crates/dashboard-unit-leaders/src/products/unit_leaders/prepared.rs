//! Leader careers are append-only facts plus the production scan's compact continuation.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use dashboard_text::metadata::check_cancel;
use dashboard_text::triggers::inputs::TableChanges;

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    series: Accumulator,
    #[serde(skip)]
    published: Published,
    #[serde(skip)]
    keys: BTreeMap<String, u64>,
    #[serde(skip)]
    countries: BTreeMap<String, BTreeSet<u64>>,
    #[serde(skip)]
    admissions: BTreeMap<String, BTreeSet<(u64, u32, u32)>>,
    #[serde(skip)]
    kinds: BTreeMap<u64, BTreeSet<u8>>,
    through: i32,
    #[serde(skip)]
    content_warnings: Vec<String>,
    /// The content refusals this family's own reads produced, which its
    /// published warnings report.
    #[serde(default)]
    refusals: dashboard_text::catalogue::OwnRefusals,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Metadata {
    countries: BTreeMap<String, BTreeSet<u64>>,
    warnings: Vec<String>,
    admissions: BTreeMap<String, BTreeSet<(u64, u32, u32)>>,
    kinds: BTreeMap<u64, BTreeSet<u8>>,
    published: Published,
}

/// The value each battle outcome and naval result source key holds in the
/// published series, as its latest event wrote it; the catalogue that names
/// those keys carries it. Observation restores the keys a frame reaches from
/// these, and publication compares against them, so neither reads the series
/// back.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Published {
    #[serde(with = "dashboard_core::map_pairs")]
    outcomes: BattleOutcomes,
    #[serde(with = "dashboard_core::map_pairs")]
    sources: BTreeMap<u64, NavalSource>,
}

/// Put `key`'s published value into the window's `current` values unless
/// the window already holds one.
fn restore<K: Ord + Copy, V: Copy>(
    current: &mut BTreeMap<K, V>,
    published: &BTreeMap<K, V>,
    key: K,
) {
    if let std::collections::btree_map::Entry::Vacant(entry) = current.entry(key)
        && let Some(value) = published.get(&key)
    {
        entry.insert(*value);
    }
}

#[derive(Serialize, Deserialize)]
enum Event {
    Run(UnitLeaderRun),
    Trait(LeaderTraitGain),
    Command(LeaderCommand),
    Battle((i32, i32, bool, u8, u64, u8)),
    Observation(LeaderObservation),
}
impl Prepared {
    pub fn new(replay: &Replay, branch: u64) -> Self {
        Self {
            scan: Scan::new(replay, branch).expect("prepared branch has recorded frames"),
            ..Default::default()
        }
    }
    pub fn read(
        reader: impl io::Read,
        keys: BTreeMap<String, u64>,
        metadata: Metadata,
    ) -> io::Result<Self> {
        let mut state: Self = serde_json::from_reader(zstd::stream::Decoder::new(reader)?)?;
        state.keys = keys;
        state.countries = metadata.countries;
        state.content_warnings = metadata.warnings;
        state.admissions = metadata.admissions;
        state.kinds = metadata.kinds;
        state.published = metadata.published;
        state.scan.leader_indices = state
            .scan
            .leaders
            .iter()
            .enumerate()
            .map(|(index, leader)| (leader.reference, index))
            .collect();
        state.scan.recount_pending();
        Ok(state)
    }
    pub fn saved_bytes(&self) -> io::Result<Vec<u8>> {
        dashboard_base::prepared::series::compress(serde_json::to_vec(self)?.as_slice(), 3)
    }
    fn key(&mut self, name: String) -> io::Result<u64> {
        if let Some(&key) = self.keys.get(&name) {
            return Ok(key);
        }
        let key = u64::try_from(self.keys.len())
            .ok()
            .and_then(|count| count.checked_add(1))
            .ok_or_else(|| dashboard_core::invalid("Leader event key overflow"))?;
        self.keys.insert(name, key);
        Ok(key)
    }
    fn event(&mut self, reference: u64, event: Event) -> io::Result<()> {
        let key = self.key(reference.to_string())?;
        self.series.event(key, self.through, &event)
    }
    /// Runs `run` as this family's own consumer of the shared content
    /// ([`dashboard_text::catalogue::own_refusals`]).
    fn own<T>(&mut self, run: impl FnOnce(&mut Self) -> T) -> T {
        let mut own = std::mem::take(&mut self.refusals);
        let value = dashboard_text::catalogue::own_refusals(&mut own, || run(self));
        self.refusals = own;
        value
    }
    /// Restore only the published outcome/source keys reached by this frame,
    /// from [`Published`]: the published series is never read while
    /// observing.
    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        content: Option<&Content>,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.own(|state| state.observe_own(frame, replay, content, changes, changes, None))
            .map(drop)
    }
    /// [`Self::observe`] of an hour of a block read by its changes, on a
    /// frame kept from it (`frame`), whose rows differ from the hour's whole
    /// frame only as `reduced` says; `tables` says what changed since the
    /// hour before. False when the hour must be observed on its whole frame
    /// instead, having changed only what that observation changes the same
    /// way.
    pub fn observe_kept(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        content: Option<&Content>,
        changes: &FrameChanges,
        tables: &dyn TableChanges,
        reduced: &dyn TableChanges,
    ) -> io::Result<bool> {
        let kept = Kept::new(reduced);
        self.own(|state| state.observe_own(frame, replay, content, changes, tables, Some(&kept)))
    }
    /// Carry the family through the quiet hours `first..=last` that follow
    /// its last observed hour, at which `tables` says what may have changed
    /// since the hour before `first`, and `rows` holds the rows of the tables
    /// they leave unchanged: what observing each of them would do. False,
    /// having changed nothing, when an hour of them must be observed.
    pub fn advance_quiet(
        &mut self,
        first: i32,
        last: i32,
        tables: &dyn TableChanges,
        rows: &DecodedFrame,
        content: Option<&Content>,
    ) -> io::Result<bool> {
        if !self.own(|state| state.scan.advance_quiet(first, last, tables, rows, content)) {
            return Ok(false);
        }
        self.series.advance_span(first, last)?;
        self.through = last;
        Ok(true)
    }
    fn observe_own(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        content: Option<&Content>,
        changes: &FrameChanges,
        tables: &dyn TableChanges,
        kept: Option<&Kept<'_>>,
    ) -> io::Result<bool> {
        self.series.advance(frame.hour)?;
        let naval_changed = [
            TableId::NavalResults,
            TableId::NavalCombatResultDetails,
            TableId::NavalResultSources,
        ]
        .into_iter()
        .any(|table| changes.rows_changed[table.index()]);
        if naval_changed && let Ok(results) = frame.complete::<tables::NavalResults>() {
            #[cfg(test)]
            {
                self.work.row_visits += results.len();
            }
            for row in results {
                restore(
                    &mut self.scan.naval_sources,
                    &self.published.sources,
                    row.id,
                );
            }
        }
        if naval_changed && let Ok(results) = frame.complete::<tables::NavalCombatResultDetails>() {
            #[cfg(test)]
            {
                self.work.row_visits += results.len();
            }
            for row in results {
                restore(
                    &mut self.scan.naval_sources,
                    &self.published.sources,
                    row.result,
                );
            }
        }
        if changes.rows_changed[TableId::LandCombatResults.index()]
            && let Ok(results) = frame.complete::<tables::LandCombatResults>()
        {
            #[cfg(test)]
            {
                self.work.row_visits += results.len();
            }
            for row in results {
                restore(
                    &mut self.scan.outcomes,
                    &self.published.outcomes,
                    (1, row.combat),
                );
            }
        }
        if naval_changed && let Ok(results) = frame.complete::<tables::NavalResults>() {
            #[cfg(test)]
            {
                self.work.row_visits += results.len();
            }
            for row in results {
                if let Some(source) = self.scan.naval_sources.get(&row.id) {
                    restore(
                        &mut self.scan.outcomes,
                        &self.published.outcomes,
                        (2, source.combat),
                    );
                }
            }
        }
        // A source reached in this very frame may introduce a previously seen combat.
        if naval_changed && let Ok(sources) = frame.complete::<tables::NavalResultSources>() {
            #[cfg(test)]
            {
                self.work.row_visits += sources.len();
            }
            for source in sources
                .iter()
                .filter(|source| !source.transient && !source.merged)
            {
                restore(
                    &mut self.scan.outcomes,
                    &self.published.outcomes,
                    (2, source.combat),
                );
            }
        }
        self.scan.reuse_evaluations = true;
        let params = full_params(&self.scan.reply.branch_id);
        let countries = &mut self.countries;
        let admissions = &mut self.admissions;
        let kinds = &mut self.kinds;
        #[cfg(test)]
        let work = &mut self.work;
        let observed = self.scan.observe_changes(
            frame,
            replay,
            &params,
            content,
            changes,
            tables,
            kept,
            &mut |reference, role, holder| {
                #[cfg(test)]
                {
                    work.history_visits += 1;
                }
                let added = kinds.entry(reference).or_default().insert(role.kind);
                #[cfg(test)]
                {
                    work.emitted += usize::from(added);
                }
                #[cfg(not(test))]
                let _ = added;
                if let Some(owner) = &role.owner {
                    let added = countries
                        .entry(owner.clone())
                        .or_default()
                        .insert(reference);
                    #[cfg(test)]
                    {
                        work.emitted += usize::from(added);
                    }
                    #[cfg(not(test))]
                    let _ = added;
                    if let Some((slot, country)) = holder {
                        let added = admissions
                            .entry(owner.clone())
                            .or_default()
                            .insert((reference, slot, country));
                        #[cfg(test)]
                        {
                            work.emitted += usize::from(added);
                        }
                        #[cfg(not(test))]
                        let _ = added;
                    }
                }
            },
        )?;
        if observed {
            self.through = frame.hour;
        }
        Ok(observed)
    }
    pub fn buffered_bytes(&self) -> usize {
        self.series.bytes().saturating_add(self.scan.pending_bytes)
    }
    /// Publish the window. Changed outcomes and sources are found against
    /// [`Published`], so the published series is never read.
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        content: Option<&Content>,
        content_errors: &[String],
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.own(|state| state.publish_own(writer, branch, content, content_errors, cancelled))?;
        // The saved state includes this flush's own refusals.
        writer.write_at(
            &format!("unit-leaders-{branch}"),
            0,
            self.saved_bytes()?.as_slice(),
        )
    }
    fn publish_own(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        content: Option<&Content>,
        content_errors: &[String],
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.seal();
        // Static presentation uses the existing content loaders once per new token.
        for token in &self.scan.token_list[self.scan.reply.traits.len()..] {
            let info = trait_info(token, content);
            if let Some(reason) = &info.missing {
                self.scan.refusals.note(reason, token);
            }
            self.scan.reply.traits.push(info);
        }
        for reference in self
            .scan
            .leaders
            .iter()
            .map(|leader| leader.reference)
            .collect::<Vec<_>>()
        {
            self.key(reference.to_string())?;
        }
        let mut events = Vec::new();
        let mut runs = Vec::new();
        for leader in &mut self.scan.leaders {
            check_cancel(cancelled)?;
            let reference = leader.reference;
            let mut chain = leader.last_run;
            while let Some(index) = chain {
                let stored = &mut self.scan.stored_runs[index];
                if Some(index) != leader.last_run && !stored.emitted {
                    events.push((reference, Event::Run(stored.run.clone())));
                    stored.emitted = true;
                }
                chain = stored.previous;
            }
            events.extend(
                std::mem::take(&mut leader.career.trait_gains)
                    .into_iter()
                    .map(|gain| (reference, Event::Trait(gain))),
            );
            let mut commands = Vec::new();
            for mut command in std::mem::take(&mut leader.career.commands) {
                if command.last_hour != self.through {
                    events.push((reference, Event::Command(command)));
                } else {
                    if command.name_unknown.len() > 1 {
                        events.push((reference, Event::Command(command.clone())));
                        let last = command.name_unknown.pop().expect("unknown command spans");
                        command.name_unknown = vec![last];
                    }
                    commands.push(command);
                }
            }
            leader.career.commands = commands;
            leader.command_indices = leader
                .career
                .commands
                .iter()
                .enumerate()
                .map(|(index, command)| {
                    (
                        command
                            .formation
                            .parse()
                            .expect("recorded formation reference"),
                        index,
                    )
                })
                .collect();
            // Only current and last-known traits feed the next observation.
            let last_known = leader.last_known_run;
            let last = leader.last_run;
            leader.last_run = None;
            leader.last_known_run = None;
            for old in [last_known, last]
                .into_iter()
                .flatten()
                .collect::<BTreeSet<_>>()
            {
                let mut stored = self.scan.stored_runs[old].clone();
                stored.previous = None;
                let index = runs.len();
                if Some(old) == last_known {
                    leader.last_known_run = Some(index);
                }
                if Some(old) == last {
                    leader.last_run = Some(index);
                }
                runs.push(stored);
            }
        }
        self.scan.stored_runs = runs;
        for (reference, rows) in std::mem::take(&mut self.scan.battle_hours) {
            events.extend(rows.into_iter().map(|row| (reference, Event::Battle(row))));
        }
        self.scan.battle_indices.clear();
        for (reference, event) in events {
            self.event(reference, event)?;
        }
        let shared = self.key("observed".into())?;
        if let Some(last) = self.scan.reply.observed.pop() {
            for row in std::mem::take(&mut self.scan.reply.observed) {
                self.series
                    .event(shared, self.through, &Event::Observation(row))?;
            }
            self.scan.reply.observed.push(last);
        }
        for (outcome, result) in std::mem::take(&mut self.scan.outcomes) {
            if self.published.outcomes.get(&outcome) != Some(&result) {
                let (kind, combat) = outcome;
                self.publish(format!("outcome:{kind}:{combat}"), &result)?;
                self.published.outcomes.insert(outcome, result);
            }
        }
        for (id, source) in std::mem::take(&mut self.scan.naval_sources) {
            if self.published.sources.get(&id) != Some(&source) {
                self.publish(format!("source:{id}"), &source)?;
                self.published.sources.insert(id, source);
            }
        }
        let mut presentation = UnitLeadersReply::default();
        extend_content_warnings(&mut presentation, content);
        presentation.warnings.extend_from_slice(content_errors);
        self.content_warnings = presentation.warnings;
        self.scan.row_indices.clear();
        self.scan.active_commands.clear();
        self.scan.recount_pending();
        self.scan.formation_names.clear();
        self.series.through(self.through);
        self.series.flush(writer, &section(branch), 0, cancelled)?;
        series::write_catalogue(
            writer,
            "unit-leaders",
            branch,
            &self.keys,
            &Metadata {
                countries: self.countries.clone(),
                warnings: self.content_warnings.clone(),
                admissions: self.admissions.clone(),
                kinds: self.kinds.clone(),
                published: self.published.clone(),
            },
        )
    }
    /// Append `value` as `name`'s latest event.
    fn publish(&mut self, name: String, value: &impl Serialize) -> io::Result<()> {
        let value = serde_json::to_value(value)?;
        let key = self.key(name)?;
        self.series.event(key, self.through, &value)
    }
}
fn full_params(branch: &str) -> UnitLeadersParams {
    UnitLeadersParams {
        branch_id: branch.into(),
        selection: LeaderSelection::All,
        view: LeaderView::Career,
        leaders: LeaderKinds::All,
    }
}
pub fn section(branch: u64) -> String {
    format!("unit-leaders-events-{branch}")
}

/// An isolated hour supplies current leader facts, never cumulative battles
/// or the dates of holdings. The ordinary observer still owns visibility,
/// content resolution and native identity joins.
pub fn at_hour(
    source: &Source,
    params: &UnitLeadersParams,
    hour: i32,
    context: &LoadContext,
    history: Option<&dashboard_base::products::country_history::RecordedHistories>,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let branch = params.branch_id.parse().expect("validated branch");
    let replay = source.replay_ref();
    let mut scan = Scan::new(replay, branch)?;
    let cat = context
        .content()
        .and_then(|options| catalogue::catalogue(source, options));
    let content = Content::for_catalogue(source, context, cat.ok())?;
    let sampled = UnitLeadersParams {
        selection: match &params.selection {
            LeaderSelection::Countries { .. } => LeaderSelection::All,
            selection => selection.clone(),
        },
        ..params.clone()
    };
    if let Some(frame) = dashboard_base::prepared::query::progressive::frame_at(
        source,
        branch,
        hour,
        &scan_tables(false, false),
        cancelled,
    )? {
        let changes = dashboard_base::prepared::frame_changes::FrameChangeTracker::new()
            .observe(&frame, replay.format);
        scan.observe_changes(
            &frame,
            replay,
            &sampled,
            content.as_ref(),
            &changes,
            &changes,
            None,
            &mut |_, _, _| {},
        )?;
    }
    let mut reply = scan.finish(&sampled, content.as_ref())?;
    if matches!(params.selection, LeaderSelection::Countries { .. }) {
        reply = project(&reply, params, history);
    }
    for leader in &mut reply.leaders {
        leader.trait_gains.clear();
        leader.commands.clear();
        leader.battle_starts = None;
        leader.battle_start_read_hours = None;
        leader.unknown_battle_starts = None;
        leader.battle_won = None;
        leader.battle_result_hours = None;
        leader.battle_result_read_hours = None;
        leader.fought_hours = None;
        leader.won_hours = None;
        leader.lost_hours = None;
        leader.partial_from = None;
    }
    reply.preparing = vec!["battles".into(), "history".into()];
    serde_json::to_value(reply).map_err(Into::into)
}

/// The prepared path uses the original materializer and selection/view projection.
pub fn query(
    store: &store::Store,
    key: &store::SourceKey,
    params: &UnitLeadersParams,
    history: Option<&dashboard_base::products::country_history::RecordedHistories>,
    cancelled: &AtomicBool,
) -> io::Result<Option<UnitLeadersReply>> {
    let branch = params.branch_id.parse().expect("validated leader branch");
    let Some((keys, metadata)) =
        series::read_catalogue::<Metadata>(store, key, "unit-leaders", branch)?
    else {
        return Ok(None);
    };
    let Some(file) = store.read(key, &format!("unit-leaders-{branch}"), 0)? else {
        return Ok(None);
    };
    let state = Prepared::read(io::BufReader::new(file), keys, metadata)?;
    let mut wanted: BTreeSet<_> = match &params.selection {
        LeaderSelection::All => state
            .scan
            .leaders
            .iter()
            .map(|leader| leader.reference)
            .collect(),
        LeaderSelection::Leader { reference } => {
            [reference.parse().expect("validated leader reference")]
                .into_iter()
                .collect()
        }
        LeaderSelection::Countries { tags } => {
            let aliases = history.map_or_else(
                || tags.clone(),
                |history| {
                    dashboard_base::products::country_history::selected_aliases(history, tags)
                },
            );
            aliases
                .iter()
                .filter_map(|tag| state.countries.get(tag))
                .flatten()
                .copied()
                .collect()
        }
    };
    let choices = params.view == LeaderView::Choices;
    let mut choice_tags = None;
    if choices {
        if let LeaderSelection::Countries { tags } = &params.selection {
            let aliases = history.map_or_else(
                || tags.clone(),
                |history| {
                    dashboard_base::products::country_history::selected_aliases(history, tags)
                },
            );
            if let Some(history) = history {
                let identities =
                    dashboard_base::products::country_history::selected_identities(history, tags);
                wanted = aliases
                    .iter()
                    .filter_map(|tag| state.admissions.get(tag))
                    .flatten()
                    .filter(|(_, slot, country)| identities.contains(&(*slot, *country)))
                    .map(|(leader, _, _)| *leader)
                    .collect();
            }
            choice_tags = Some(
                aliases
                    .into_iter()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>(),
            );
        }
        wanted.retain(|leader| {
            state
                .kinds
                .get(leader)
                .is_some_and(|kinds| kinds.iter().any(|kind| params.leaders.includes(*kind)))
        });
    }
    let mut reader = Reader::new(store, key, section(branch), cancelled)?;
    let mut scan = state.scan;
    scan.reply.branch_id.clone_from(&params.branch_id);
    scan.reply.warnings.extend(state.content_warnings);
    scan.leaders
        .retain(|leader| wanted.contains(&leader.reference));
    let current_observed = std::mem::take(&mut scan.reply.observed);
    if !choices && let Some(&shared) = state.keys.get("observed") {
        for event in events(&mut reader, shared, cancelled)? {
            match event {
                Event::Observation(row) => scan.reply.observed.push(row),
                _ => return Err(dashboard_core::invalid("Invalid shared leader observation")),
            }
        }
    }
    scan.reply.observed.extend(current_observed);
    if choices {
        // Current facts are the original full scan's last run. Admission is a
        // native-holder/kind roster, independent of the length of the career.
        let full = scan.finish(&full_params(&params.branch_id), None)?;
        let choice_params = UnitLeadersParams {
            selection: match &params.selection {
                LeaderSelection::Leader { reference } => LeaderSelection::Leader {
                    reference: reference.clone(),
                },
                _ => LeaderSelection::All,
            },
            leaders: LeaderKinds::All,
            ..params.clone()
        };
        let mut reply = project(&full, &choice_params, None);
        reply.selection_tags = choice_tags;
        reply_bytes(&reply)?;
        return Ok(Some(reply));
    }
    let mut materialized = Vec::new();
    for leader in &mut scan.leaders {
        check_cancel(cancelled)?;
        let mut runs = BTreeMap::<i32, UnitLeaderRun>::new();
        if let Some(index) = leader.last_run {
            let row = scan.stored_runs[index].run.clone();
            runs.insert(row.first_hour, row);
        }
        let mut commands = BTreeMap::<(i32, u64), LeaderCommand>::new();
        let current_commands = std::mem::take(&mut leader.career.commands);
        let mut battles = BTreeMap::<(u8, u64), (i32, i32, bool, u8, u64, u8)>::new();
        if let Some(&id) = state.keys.get(&leader.reference.to_string()) {
            for event in events(&mut reader, id, cancelled)? {
                match event {
                    Event::Run(run) => {
                        runs.insert(run.first_hour, run);
                    }
                    Event::Trait(gain) => leader.career.trait_gains.push(gain),
                    Event::Command(command) => merge_command(&mut commands, command),
                    Event::Battle(row) => {
                        battles
                            .entry((row.3, row.4))
                            .and_modify(|before| before.5 |= row.5)
                            .or_insert(row);
                    }
                    _ => return Err(dashboard_core::invalid("Invalid leader career event")),
                }
            }
        }
        for command in current_commands {
            merge_command(&mut commands, command);
        }
        leader.career.commands = commands.into_values().collect();
        leader.last_run = None;
        for run in runs.into_values() {
            let index = materialized.len();
            materialized.push(StoredRun {
                run,
                previous: leader.last_run,
                emitted: false,
            });
            leader.last_run = Some(index);
        }
        let rows: Vec<_> = battles.into_values().collect();
        for row in &rows {
            restore(
                &mut scan.outcomes,
                &state.published.outcomes,
                (row.3, row.4),
            );
        }
        scan.battle_hours.insert(leader.reference, rows);
    }
    scan.stored_runs = materialized;
    let full = scan.finish(&full_params(&params.branch_id), None)?;
    let reply = project(&full, params, history);
    reply_bytes(&reply)?;
    Ok(Some(reply))
}
fn events(reader: &mut Reader<'_>, key: u64, cancelled: &AtomicBool) -> io::Result<Vec<Event>> {
    let mut result = Vec::new();
    let mut cursor = 0;
    loop {
        check_cancel(cancelled)?;
        // This bounds one disk read, never the complete reachable history.
        let page = reader.events(&[key], i32::MIN, i32::MAX, cursor, 256)?;
        if page
            .cursor
            .is_some_and(|next| next <= cursor || page.rows.is_empty())
        {
            return Err(dashboard_core::invalid(
                "Leader career event page made no progress",
            ));
        }
        for row in page.rows {
            result.push(serde_json::from_value(row.row)?);
        }
        if let Some(next) = page.cursor {
            cursor = next;
        } else {
            break;
        }
    }
    Ok(result)
}
fn merge_command(commands: &mut BTreeMap<(i32, u64), LeaderCommand>, mut command: LeaderCommand) {
    let key = (
        command.first_hour,
        command
            .formation
            .parse::<u64>()
            .expect("recorded formation reference"),
    );
    if let Some(before) = commands.get_mut(&key) {
        before.last_hour = before.last_hour.max(command.last_hour);
        if before.name.is_none() {
            before.name = command.name.take();
            before.name_read_hour = command.name_read_hour;
        }
        for span in command.name_unknown {
            if let Some(last) = before
                .name_unknown
                .last_mut()
                .filter(|last| span.first_hour <= last.last_hour.saturating_add(1))
            {
                last.last_hour = last.last_hour.max(span.last_hour);
            } else {
                before.name_unknown.push(span);
            }
        }
    } else {
        commands.insert(key, command);
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
#[cfg(test)]
mod tests;
