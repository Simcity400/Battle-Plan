//! Template dictionary, every hour, with exact native text: the counter
//! inputs, the stored 3D-model inputs and the saved model choices of one
//! walk, whose sub-unit definitions resolve through caller-owned slots that
//! grow off-thread, and the names and categories derived from the same walk
//! ([`dictionary`]). Each walked template's movement speed keys
//! (DivisionTemplateIcons.speed_keys, [`engine_native::carmy::movement_speed_keys`])
//! are read after the walk from its template object; a refused read is that
//! row's None, counted in [`speed_key_data`].
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use collector_projection::projection::{Projection, project_table};
use engine_allocation::{copy_vec, owned_vec};
use engine_native::{Memory, creferenced_division_template as native, walk::Outcome};
use replay_data::rows::{
    CountryTemplateStateRow, DivisionTemplateIconRow, DivisionTemplateRow,
    GarrisonTemplateSelection, Recorded, TableId, TemplateCell, TemplateDisplayName, TemplateGrid,
    TemplateGrids, TemplateLayout, TemplateNameValue, TemplateSettings, TemplateSubunit, tables,
};
use replay_frame::frame::{TableInput, TableView};

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
    TableId::CountryTemplateState,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("division template dictionary and country census", 0, &[0]),
    ("division template counter and model inputs", 1, &[1]),
    ("country template configuration", 2, &[2]),
];

/// Cumulative DivisionTemplateIcons.speed_keys reads and refusals.
static SPEED_KEY_COUNTS: [std::sync::atomic::AtomicU64; 2] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 2];

pub(crate) fn speed_key_data() -> impl serde::Serialize {
    use std::sync::atomic::Ordering::Relaxed;
    crate::status::status!({
        "reads": SPEED_KEY_COUNTS[0].load(Relaxed),
        "refusals": SPEED_KEY_COUNTS[1].load(Relaxed),
        "scope": "cumulative fresh DivisionTemplateIcons.speed_keys reads of the movement priority sub-unit's and its categories' modifier keys; certified icon cores reuse complete keys without native reads, and a refused fresh read is that row's None",
    })
}

/// Distinct failure reasons one hour's summary keeps by name; the rest are
/// counted together.
pub const MODEL_REASONS: usize = 8;

/// This hour's DivisionTemplateIcons rows whose model inputs failed, by
/// reason, for status.json. Fixed size: nothing is allocated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelFailures {
    pub rows: u32,
    /// (reason, rows) in first-seen order.
    pub reasons: [(&'static str, u32); MODEL_REASONS],
    /// Rows whose reason did not fit `reasons`.
    pub other: u32,
}

impl ModelFailures {
    fn add(&mut self, reason: &'static str) {
        self.rows += 1;
        for slot in &mut self.reasons {
            if slot.1 == 0 {
                *slot = (reason, 1);
                return;
            }
            if slot.0 == reason {
                slot.1 += 1;
                return;
            }
        }
        self.other += 1;
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    bytes: usize,
    counts: usize,
    variants: usize,
    icon_rows: usize,
    country_rows: usize,
    icon_bytes: usize,
    name_bytes: usize,
    /// The sub-unit database's length, for the hourly walk's two copies.
    database: usize,
    /// The hourly walk's per-country definition slots.
    locals: usize,
}

impl Capacities {
    /// Country templates retain grouped subunits, model representatives,
    /// and the literal or fallback name. Empty outputs still visit every
    /// authenticated roster record and count its independent arena demands.
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let countries = engine_native::ccountry::Reader::new(memory, base, state)?
            .count()
            .saturating_sub(1) as usize;
        crate::table_capture::grow_capacity(&mut self.country_rows, countries);
        let daily = native::walk(memory, base, state, &mut [], &mut [], &mut []);
        let icons = native::icon_capacity(memory, base, state);
        let demand = |outcome| match outcome {
            Outcome::Rows { len } => Ok(len),
            Outcome::Truncated { len, unread_rows } => len
                .checked_add(unread_rows as usize)
                .ok_or("template row demand overflow"),
            Outcome::Unavailable { reason } => Err(reason),
        };
        let rows = demand(daily.outcome);
        let icon_rows = demand(icons.outcome);
        // This sizing pass runs on the loader before hook publication and
        // on existing sizing workers. Offer both certificate domains before
        // the first boundary, including country-only demand if templates fail.
        crate::template_changes::prepare(icon_rows.as_ref().copied().unwrap_or(0), countries);
        crate::local_subunit_changes::prepare(countries);
        let rows = rows?;
        let icon_rows = icon_rows?;
        for (capacity, required) in [
            (&mut self.rows, rows),
            (&mut self.bytes, daily.required_bytes),
            (&mut self.icon_rows, icon_rows),
            (&mut self.icon_bytes, icons.required_bytes),
            (&mut self.name_bytes, daily.required_name_bytes),
            (&mut self.counts, icons.required_counts.saturating_mul(2)),
            (&mut self.variants, icons.required_variants),
            (&mut self.database, icons.required_database),
            (&mut self.locals, icons.required_locals),
        ] {
            crate::table_capture::grow_capacity(capacity, required);
        }
        Ok(())
    }
}

struct Preacquired {
    base: usize,
    state: usize,
    hour: i32,
    force: bool,
    ticks: Option<[u64; N]>,
}
#[derive(Default, Clone)]
struct CountryRow {
    garrison_priority: Option<i32>,
    state: engine_deployment::cdeployment_status::TemplateState,
    garrisons: Option<std::sync::Arc<Vec<GarrisonTemplateSelection>>>,
    garrison_failure: Option<&'static str>,
}
pub struct Captured {
    preacquired: Option<Preacquired>,
    pending_owned: bool,
    rows: CaptureRows<native::Row>,
    bytes: Arena<u8>,
    counts: Arena<native::SubunitCount>,
    variants: Arena<u64>,
    icon_rows: CaptureRows<native::IconRow>,
    country_rows: CaptureRows<CountryRow>,
    country_cache: countries::Store,
    country_snapshot: Option<std::sync::Arc<replay_data::rows::TableRows>>,
    icon_bytes: Arena<u8>,
    name_bytes: Arena<u8>,
    sample_hour: Option<i32>,
    /// The dictionary's held name keys, kept across hours.
    keys: dictionary::KeyStore,
    held: held::Store,
    core: core::Store,
    model_failures: ModelFailures,
    database: Arena<usize>,
    globals: Arena<native::GlobalSlot>,
    locals: Arena<native::LocalSlot>,
    /// The hour's movement speed keys of every walked template, in walk order.
    speed_keys: Vec<u32>,
    /// Each walked template's span of `speed_keys`, by template, ascending;
    /// `None` is its counted refusal.
    speed_spans: Vec<(u64, Option<(usize, usize)>)>,
}
impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.pending_owned = false;
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.icon_rows
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
        // A failed capture has no rows of its own to count.
        self.model_failures = ModelFailures::default();
        self.keys.forget();
        self.held.forget();
        self.core.forget();
        self.country_cache.forget();
        self.country_snapshot = None;
        self.country_rows.fail(Outcome::Unavailable { reason });
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            bytes: 0,
            counts: 0,
            variants: 0,
            icon_rows: 0,
            country_rows: 0,
            icon_bytes: 0,
            name_bytes: 0,
            database: 0,
            locals: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            preacquired: None,
            pending_owned: false,
            rows: CaptureRows::new(capacity.rows),
            bytes: Arena::new(capacity.bytes),
            counts: Arena::new(capacity.counts),
            variants: Arena::new(capacity.variants),
            icon_rows: CaptureRows::new(capacity.icon_rows),
            country_rows: CaptureRows::new(capacity.country_rows),
            country_cache: countries::Store::default(),
            country_snapshot: None,
            icon_bytes: Arena::new(capacity.icon_bytes),
            name_bytes: Arena::new(capacity.name_bytes),
            sample_hour: None,
            keys: dictionary::KeyStore::default(),
            held: held::Store::default(),
            core: core::Store::default(),
            model_failures: ModelFailures::default(),
            database: Arena::new(capacity.database),
            globals: Arena::new(capacity.database),
            locals: Arena::new(capacity.locals),
            speed_keys: Vec::new(),
            speed_spans: Vec::new(),
        }
    }
    pub fn reset(&mut self) {
        self.discard_owned();
        self.preacquired = None;
        self.rows.reset();
        self.bytes.reset();
        self.counts.reset();
        self.variants.reset();
        self.icon_rows.reset();
        self.country_rows.reset();
        self.icon_bytes.reset();
        self.name_bytes.reset();
        self.sample_hour = None;
        self.model_failures = ModelFailures::default();
        self.database.reset();
        self.globals.reset();
        self.locals.reset();
        self.speed_keys.clear();
        self.speed_spans.clear();
    }
    fn discard_owned(&mut self) {
        if self.pending_owned {
            self.keys.forget();
            self.held.forget();
            self.core.forget();
            self.country_cache.forget();
            self.country_snapshot = None;
            self.pending_owned = false;
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.bytes.allocated_bytes()
            + self.counts.allocated_bytes()
            + self.variants.allocated_bytes()
            + self.icon_rows.allocated_bytes()
            + self.country_rows.allocated_bytes()
            + self.country_cache.allocated_bytes()
            + self.icon_bytes.allocated_bytes()
            + self.name_bytes.allocated_bytes()
            + self.keys.allocated_bytes()
            + self.held.allocated_bytes()
            + self.core.allocated_bytes()
            + self.database.allocated_bytes()
            + self.globals.allocated_bytes()
            + self.locals.allocated_bytes()
            + self.speed_keys.capacity() * size_of::<u32>()
            + self.speed_spans.capacity() * size_of::<(u64, Option<(usize, usize)>)>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        self.bytes.grow(&mut capacity.bytes);
        self.counts.grow(&mut capacity.counts);
        self.variants.grow(&mut capacity.variants);
        self.icon_rows.grow(&mut capacity.icon_rows);
        self.country_rows.grow(&mut capacity.country_rows);
        self.icon_bytes.grow(&mut capacity.icon_bytes);
        self.name_bytes.grow(&mut capacity.name_bytes);
        let mut globals = capacity.database;
        self.database.grow(&mut capacity.database);
        self.globals.grow(&mut globals);
        self.locals.grow(&mut capacity.locals);
    }

    pub(crate) fn naming_demand(&self) -> usize {
        match self.icon_rows.outcome {
            Outcome::Rows { len } => len,
            Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
            Outcome::Unavailable { .. } => 0,
        }
    }
    /// Tests: the staggered whole-pack path, which takes a matching
    /// preacquisition or reads afresh.
    #[cfg(test)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force: bool,
    ) -> [u64; N] {
        if let Some(marker) = &mut self.preacquired {
            if (marker.base, marker.state, marker.hour, marker.force) == (base, state, hour, force)
            {
                let ticks = marker.ticks.take().unwrap_or([0; N]);
                self.settle_owned();
                return ticks;
            }
            self.fail_preacquired(
                base,
                state,
                hour,
                force,
                "template preacquisition context mismatch",
            );
            return [0; N];
        }
        self.capture_original(
            memory,
            base,
            state,
            hour,
            force,
            &crate::template_changes::Observer,
        )
    }
    pub(crate) fn capture_preacquired(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force: bool,
    ) {
        let ticks = self.capture_observed(
            memory,
            base,
            state,
            hour,
            force,
            &crate::template_changes::Observer,
        );
        self.preacquired = Some(Preacquired {
            base,
            state,
            hour,
            force,
            ticks: Some(ticks),
        });
    }
    pub(crate) fn fail_preacquired(
        &mut self,
        base: usize,
        state: usize,
        hour: i32,
        force: bool,
        reason: &'static str,
    ) {
        self.fail_capture(hour, force, reason);
        self.preacquired = Some(Preacquired {
            base,
            state,
            hour,
            force,
            ticks: Some([0; N]),
        });
    }
    /// The ticks of the walk the template job preacquired,
    /// taken as [`Self::capture`] takes them, without a native read. Without
    /// a matching preacquisition the hour's templates are refused: this
    /// never walks.
    pub(crate) fn take_preacquired(
        &mut self,
        base: usize,
        state: usize,
        hour: i32,
        force: bool,
    ) -> [u64; N] {
        if let Some(marker) = &mut self.preacquired
            && (marker.base, marker.state, marker.hour, marker.force) == (base, state, hour, force)
        {
            return marker.ticks.take().unwrap_or([0; N]);
        }
        let reason = if self.preacquired.is_some() {
            "template preacquisition context mismatch"
        } else {
            "template preacquisition not supplied"
        };
        self.fail_preacquired(base, state, hour, force, reason);
        [0; N]
    }
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn capture_original(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force: bool,
        changes: &dyn crate::template_changes::Changes,
    ) -> [u64; N] {
        let mut ticks = self.capture_observed(memory, base, state, hour, force, changes);
        let start = qpc();
        self.settle_owned();
        ticks[1] += qpc().saturating_sub(start).max(0) as u64;
        ticks
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_observed(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force: bool,
        changes: &dyn crate::template_changes::Changes,
    ) -> [u64; N] {
        self.reset();
        if force {
            self.held.forget();
            self.core.forget();
            self.country_cache.forget();
            self.country_snapshot = None;
        }
        self.country_cache.begin();
        self.held.begin(changes);
        self.sample_hour = Some(hour);
        // The counter/model inputs, each composition grid's entries and the
        // demanded display-name inputs every hour.
        let mut country_ticks = 0_u64;
        let icons = {
            let _reads = crate::read_accounting::table(TABLES[1]);
            let start = qpc();
            let mut country_len = 0;
            let mut country_failure = None;
            let country_rows = &mut self.country_rows.rows;
            let country_cache = &mut self.country_cache;
            let context_failures = &mut self.model_failures;
            let mut countries = |country| {
                if country_failure.is_some() {
                    return;
                }
                let start = qpc();
                let _reads = crate::read_accounting::table(TABLES[2]);
                let result = (|| {
                    let row = country_cache.read(memory, base, state, country, changes)?;
                    if let Some(reason) = row.garrison_failure {
                        context_failures.add(reason);
                    }
                    engine_native::walk::Buffer::Growing(&mut *country_rows)
                        .ensure(country_len + 1)?;
                    country_rows[country_len] = row;
                    country_len += 1;
                    Ok::<(), &'static str>(())
                })();
                country_ticks += qpc().saturating_sub(start).max(0) as u64;
                country_failure = result.err();
            };
            let mut details = held::Demand::new(&mut self.held, &mut self.core, changes);
            let walk = native::walk_icons_with_selected_details_growing(
                memory,
                base,
                state,
                &mut self.icon_rows.rows,
                &mut self.icon_bytes.first,
                &mut self.counts.first,
                &mut self.variants.first,
                native::DefinitionSlots {
                    database: &mut self.database.first,
                    globals: &mut self.globals.first,
                    locals: &mut self.locals.first,
                },
                &mut self.name_bytes.first,
                &mut countries,
                &mut details,
            );
            self.country_rows.outcome = match (walk.outcome, country_failure) {
                (Outcome::Unavailable { reason }, _) | (_, Some(reason)) => {
                    Outcome::Unavailable { reason }
                }
                _ => Outcome::Rows { len: country_len },
            };
            crate::template_changes::record_countries(country_len);
            self.icon_rows.outcome = walk.outcome;
            self.icon_bytes.len = walk.bytes_len;
            self.icon_bytes.required = walk.required_bytes;
            self.name_bytes.len = walk.name_bytes_len;
            self.name_bytes.required = walk.required_name_bytes;
            self.counts.len = walk.counts_len;
            self.counts.required = walk.required_counts;
            self.variants.len = walk.variants_len;
            self.variants.required = walk.required_variants;
            self.database.required = walk.required_database;
            self.globals.required = walk.required_database;
            self.locals.required = walk.required_locals;
            self.capture_speed_keys(memory, base, changes);
            if let Some(rows) = self.icon_rows.rows.get(..walk.outcome.len()) {
                self.held.certify(changes, rows);
                for row in rows {
                    if !self.held.settings_available(row) {
                        self.model_failures
                            .add("template details writer certificate unavailable");
                    }
                    if let Some(reason) = row
                        .model_failure
                        .or(row.equipment_failure)
                        .or(row.details_failure)
                    {
                        self.model_failures.add(reason);
                    }
                }
            }
            qpc().saturating_sub(start).max(0) as u64
        };
        let country_native_ticks = country_ticks;
        self.country_cache.finish();
        self.pending_owned = true;
        // The dictionary, from the same walk.
        let _reads = crate::read_accounting::table(TABLES[0]);
        let start = qpc();
        let icon_rows = self
            .icon_rows
            .rows
            .get(..self.icon_rows.outcome.len())
            .unwrap_or(&[]);
        let names = self
            .name_bytes
            .first
            .get(..self.name_bytes.len)
            .unwrap_or(&[]);
        self.rows.outcome = dictionary::derive(
            memory,
            icon_rows,
            self.icon_rows.outcome,
            names,
            &mut self.keys,
            changes,
            crate::sampled_verify::current_burned_in(crate::template_changes::VERIFY),
            &mut self.rows.rows,
            &mut self.bytes.first,
        );
        self.bytes.len = self.bytes.first.len();
        self.bytes.required = self.bytes.len;
        [
            qpc().saturating_sub(start).max(0) as u64,
            icons.saturating_sub(country_native_ticks),
            country_ticks,
        ]
    }

    /// Owned observations and frozen `held` certificates; no Memory capability.
    pub(crate) fn settle_owned(&mut self) {
        if !self.pending_owned {
            return;
        }
        let rows = self
            .icon_rows
            .rows
            .get(..self.icon_rows.outcome.len())
            .unwrap_or(&[]);
        self.held.settle(
            rows,
            &self.counts.first[..self.counts.len],
            &self.variants.first[..self.variants.len],
            &self.icon_bytes.first[..self.icon_bytes.len],
        );
        if matches!(self.icon_rows.outcome, Outcome::Rows { .. }) {
            self.held.prune(rows);
            self.core.prune(rows);
        }
        self.speed_spans.sort_unstable_by_key(|entry| entry.0);
        if matches!(self.country_rows.outcome, Outcome::Rows { .. }) {
            self.country_cache.prune();
        } else {
            self.country_snapshot = None;
        }
        if let Outcome::Rows { len } = self.country_rows.outcome
            && (self.country_cache.changed
                || self
                    .country_snapshot
                    .as_ref()
                    .is_none_or(|rows| rows.len() != len))
            && let TableView::Complete(rows) = self.country_state()
        {
            self.country_snapshot = Some(std::sync::Arc::new(rows));
        }
        if crate::allocation::failed() {
            self.discard_owned();
        } else {
            self.pending_owned = false;
        }
    }
    /// DivisionTemplateIcons.speed_keys of every walked template, read from
    /// its template object after the walk.
    fn capture_speed_keys(
        &mut self,
        memory: &impl Memory,
        base: usize,
        changes: &dyn crate::template_changes::Changes,
    ) {
        self.speed_keys.clear();
        self.speed_spans.clear();
        let rows = self
            .icon_rows
            .rows
            .get(..self.icon_rows.outcome.len())
            .unwrap_or(&[]);
        let mut counts = [0_u64; 2];
        if engine_allocation::reserve(&mut self.speed_spans, rows.len()).is_err() {
            // No span to publish: every row refuses.
            counts[1] = rows.len() as u64;
        } else {
            for row in rows {
                let start = self.speed_keys.len();
                let span = if let Some(keys) = self.core.speed(row.template, changes) {
                    let span = engine_allocation::reserve(&mut self.speed_keys, keys.len())
                        .ok()
                        .map(|()| {
                            self.speed_keys.extend_from_slice(keys);
                            (start, keys.len())
                        });
                    counts[1] += u64::from(span.is_none());
                    span
                } else {
                    let span = engine_native::carmy::movement_speed_keys(
                        memory,
                        base,
                        row.pointer,
                        &mut self.speed_keys,
                    )
                    .ok()
                    .map(|()| (start, self.speed_keys.len() - start));
                    counts[usize::from(span.is_none())] += 1;
                    self.core.remember_speed(
                        row.template,
                        span.map(|(start, len)| &self.speed_keys[start..start + len]),
                        changes,
                    );
                    span
                };
                self.speed_spans.push((row.template, span));
            }
        }
        for (total, count) in SPEED_KEY_COUNTS.iter().zip(counts) {
            total.fetch_add(count, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// The template's speed keys this hour; `None` is its refusal.
    fn speed_keys(&self, template: u64) -> Option<Vec<u32>> {
        self.speed_key_slice(template)
            .map(|keys| owned_vec(copy_vec(keys)))
    }

    fn speed_key_slice(&self, template: u64) -> Option<&[u32]> {
        let at = self
            .speed_spans
            .binary_search_by_key(&template, |entry| entry.0)
            .ok()?;
        let (start, len) = self.speed_spans[at].1?;
        self.speed_keys.get(start..start.checked_add(len)?)
    }

    /// This hour's rows whose model inputs failed, by reason.
    pub(crate) fn model_failures(&self) -> ModelFailures {
        self.model_failures
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.rows.outcome,
            self.icon_rows.outcome,
            self.country_rows.outcome,
        ]
    }
    #[cfg(test)]
    pub fn tables(&self) -> [TableView; 2] {
        let [dictionary, icons, _] = self.table_inputs();
        [dictionary, icons].map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("template conversion returns sampled views"),
        })
    }

    /// A display name equal to this frame's complete dictionary row is a
    /// same-frame certificate; a failed or truncated dictionary leaves every
    /// name a self-contained Changed override.
    #[cfg(test)]
    pub(crate) fn table_inputs(&self) -> [TableInput; N] {
        self.projected_inputs(None)
    }

    pub(crate) fn projected_inputs(
        &self,
        projection: Option<&mut Projection<'_>>,
    ) -> [TableInput; N] {
        let dictionary = self.templates();
        let names = match &dictionary {
            TableView::Complete(rows) => rows.rows::<tables::DivisionTemplates>(),
            _ => None,
        };
        let icons = self.icons(projection, names);
        [
            TableInput::Sampled(dictionary),
            icons,
            if matches!(self.country_rows.outcome, Outcome::Rows { .. }) {
                self.country_snapshot
                    .as_ref()
                    .map(|rows| TableInput::SharedComplete(std::sync::Arc::clone(rows)))
                    .unwrap_or_else(|| TableInput::Sampled(self.country_state()))
            } else {
                TableInput::Sampled(self.country_state())
            },
        ]
    }

    fn country_state(&self) -> TableView {
        table::<tables::CountryTemplateState, _>(
            self.country_rows.outcome,
            &self.country_rows.rows,
            |r| CountryTemplateStateRow {
                country: r.state.country,
                selected_staff: r.state.selected_staff,
                next_hq_order: r.state.next_hq_order,
                lock_all: r.state.lock_all,
                garrison_priority: Recorded::Value(r.garrison_priority),
                garrisons: Recorded::Value(r.garrisons.clone()),
            },
        )
    }

    fn templates(&self) -> TableView {
        let outcome = self.rows.outcome;
        let Some(bytes) = self.bytes.first.get(..self.bytes.len) else {
            return unavailable("template byte arena invalid");
        };
        let Some(rows) = self.rows.rows.get(..outcome.len()) else {
            return unavailable("template row length invalid");
        };
        if rows
            .iter()
            .any(|r| span(bytes, r.name).is_none() || span(bytes, r.name_key).is_none())
        {
            return unavailable("template row spans outside captured arenas");
        }
        table::<tables::DivisionTemplates, _>(outcome, &self.rows.rows, |r| DivisionTemplateRow {
            template: r.template,
            owner: r.owner,
            name: owned_vec(copy_vec(span(bytes, r.name).unwrap())),
            name_key: replay_data::rows::Recorded::Value(owned_vec(copy_vec(
                span(bytes, r.name_key).unwrap(),
            ))),
            unit_type: r.unit_type,
        })
    }

    fn icons(
        &self,
        projection: Option<&mut Projection<'_>>,
        dictionary: Option<&[DivisionTemplateRow]>,
    ) -> TableInput {
        let Some(names) = self.name_bytes.first.get(..self.name_bytes.len) else {
            return unavailable("template display name arena invalid").into();
        };
        let outcome = self.icon_rows.outcome;
        let Some(bytes) = self.icon_bytes.first.get(..self.icon_bytes.len) else {
            return unavailable("template icon byte arena invalid").into();
        };
        let Some(counts) = self.counts.first.get(..self.counts.len) else {
            return unavailable("template grouped arena invalid").into();
        };
        let Some(variants) = self.variants.first.get(..self.variants.len) else {
            return unavailable("template variant arena invalid").into();
        };
        let Some(rows) = self.icon_rows.rows.get(..outcome.len()) else {
            return unavailable("template icon row length invalid").into();
        };
        if rows.iter().any(|r| {
            span(bytes, r.priority_unit_name).is_none()
                || span(bytes, r.model_entity).is_none()
                || span(counts, r.counts).is_none()
                || r.grids.iter().any(|grid| span(counts, *grid).is_none())
                || r.cells.iter().any(|grid| span(counts, *grid).is_none())
                || span(variants, r.excluded_variants).is_none()
                || span(variants, r.forbidden_equipment).is_none()
        }) {
            return unavailable("template icon row spans outside captured arenas").into();
        }
        let entries = |range: native::Span| {
            owned_vec(engine_allocation::collect_vec(
                span(counts, range)
                    .unwrap()
                    .iter()
                    .map(|unit| TemplateSubunit {
                        token: unit.token,
                        local: u8::from(unit.local),
                        count: unit.count,
                    }),
            ))
        };
        let base_width = |r: &native::IconRow| {
            (r.model_failure.is_none())
                .then(|| {
                    span(counts, r.counts)?.iter().try_fold(0_i64, |sum, unit| {
                        Some(
                            sum.wrapping_add(
                                i64::from(unit.count)
                                    .wrapping_mul(100_000)
                                    .wrapping_mul(unit.width?)
                                    / 100_000,
                            ),
                        )
                    })
                })
                .flatten()
        };
        let same_entries = |range: native::Span, previous: &[TemplateSubunit]| {
            let current = span(counts, range).unwrap();
            current.len() == previous.len()
                && current.iter().zip(previous).all(|(a, b)| {
                    a.token == b.token && u8::from(a.local) == b.local && a.count == b.count
                })
        };
        project_table::<tables::DivisionTemplateIcons, _>(
            projection,
            outcome,
            &self.icon_rows.rows,
            |r| u128::from(r.template),
            |r, previous| {
                r.priority_unit == previous.priority_unit
                    && span(bytes, r.priority_unit_name).unwrap() == previous.priority_unit_name
                    && r.custom_icon == previous.custom_icon
                    && r.custom_icon_nato == previous.custom_icon_nato
                    && r.representative.token == previous.representative
                    && u8::from(r.representative.local) == previous.representative_local
                    && same_entries(r.counts, &previous.subunits)
                    && r.country == previous.country
                    && r.foreign_country == previous.foreign_country
                    && r.model_category == previous.model_category
                    && span(bytes, r.model_entity).unwrap() == previous.model_entity
                    && span(variants, r.excluded_variants).unwrap() == previous.excluded_variants
                    && u8::from(r.model_failure.is_some()) == previous.model_unreadable
                    && Recorded::Value(r.formation_division_count)
                        == previous.formation_division_count
                    && same_name_inputs(
                        r,
                        names,
                        dictionary,
                        self.sample_hour,
                        &previous.display_name,
                    )
                    && match &previous.grids {
                        Recorded::Value(grids) => {
                            same_entries(r.grids[0], &grids.regiments)
                                && same_entries(r.grids[1], &grids.support)
                                && same_entries(r.grids[2], &grids.regimental_support)
                        }
                        Recorded::NotRecorded => false,
                    }
                    && Recorded::Value(r.manpower_exile) == previous.manpower_exile
                    && Recorded::Value(base_width(r)) == previous.base_width
                    && self
                        .held
                        .matches_values(r, &previous.layout, &previous.settings)
                    && match &previous.speed_keys {
                        Recorded::Value(keys) => {
                            self.speed_key_slice(r.template) == keys.as_deref()
                        }
                        Recorded::NotRecorded => false,
                    }
            },
            |r| {
                let [regiments, support, regimental_support] = r.grids;
                let (layout, settings) = self.held.values(r);
                DivisionTemplateIconRow {
                    manpower_exile: Recorded::Value(r.manpower_exile),
                    template: r.template,
                    base_width: Recorded::Value(base_width(r)),
                    priority_unit: r.priority_unit,
                    priority_unit_name: owned_vec(copy_vec(
                        span(bytes, r.priority_unit_name).unwrap(),
                    )),
                    custom_icon: r.custom_icon,
                    custom_icon_nato: r.custom_icon_nato,
                    representative: r.representative.token,
                    representative_local: u8::from(r.representative.local),
                    subunits: entries(r.counts),
                    country: r.country,
                    foreign_country: r.foreign_country,
                    model_category: r.model_category,
                    model_entity: owned_vec(copy_vec(span(bytes, r.model_entity).unwrap())),
                    excluded_variants: owned_vec(copy_vec(
                        span(variants, r.excluded_variants).unwrap(),
                    )),
                    model_unreadable: u8::from(r.model_failure.is_some()),
                    formation_division_count: replay_data::rows::Recorded::Value(
                        r.formation_division_count,
                    ),
                    display_name: name_inputs(r, names, dictionary, self.sample_hour),
                    grids: Recorded::Value(TemplateGrids {
                        regiments: entries(regiments),
                        support: entries(support),
                        regimental_support: entries(regimental_support),
                    }),
                    layout,
                    settings,
                    speed_keys: Recorded::Value(self.speed_keys(r.template)),
                }
            },
        )
    }
}

/// Exact byte comparison, not a digest or pointer identity. Dictionary rows are
/// sorted by the existing table conversion. New handles and changed values need
/// no daily row; overrides make keyframes and direct seeks self-contained.
enum NameValue<'a> {
    Same,
    Changed {
        literal: &'a [u8],
        fallback_key: Option<&'a [u8]>,
    },
}

fn name_inputs(
    row: &native::IconRow,
    bytes: &[u8],
    dictionary: Option<&[DivisionTemplateRow]>,
    hour: Option<i32>,
) -> Recorded<Option<TemplateDisplayName>> {
    Recorded::Value(
        name_value(row, bytes, dictionary, hour).map(|value| TemplateDisplayName {
            sample_hour: replay_data::rows::PacketHour::Host,
            value: match value {
                NameValue::Same => TemplateNameValue::SameAsDivisionTemplates,
                NameValue::Changed {
                    literal,
                    fallback_key,
                } => TemplateNameValue::Changed {
                    literal: owned_vec(copy_vec(literal)),
                    fallback_key: fallback_key.map(|key| owned_vec(copy_vec(key))),
                },
            },
        }),
    )
}

fn same_name_inputs(
    row: &native::IconRow,
    bytes: &[u8],
    dictionary: Option<&[DivisionTemplateRow]>,
    hour: Option<i32>,
    previous: &Recorded<Option<TemplateDisplayName>>,
) -> bool {
    match (name_value(row, bytes, dictionary, hour), previous) {
        (None, Recorded::Value(None)) => true,
        (Some(value), Recorded::Value(Some(previous)))
            if previous.sample_hour == replay_data::rows::PacketHour::Host =>
        {
            match (value, &previous.value) {
                (NameValue::Same, TemplateNameValue::SameAsDivisionTemplates) => true,
                (
                    NameValue::Changed {
                        literal,
                        fallback_key,
                    },
                    TemplateNameValue::Changed {
                        literal: old_literal,
                        fallback_key: old_key,
                    },
                ) => literal == old_literal && fallback_key == old_key.as_deref(),
                _ => false,
            }
        }
        _ => false,
    }
}

fn name_value<'a>(
    row: &native::IconRow,
    bytes: &'a [u8],
    dictionary: Option<&[DivisionTemplateRow]>,
    hour: Option<i32>,
) -> Option<NameValue<'a>> {
    if hour.is_none() {
        return None;
    }
    let name = row.display_name?;
    let literal = span(bytes, name.literal)?;
    let key = match name.fallback_key {
        Some(range) => match span(bytes, range) {
            Some(bytes) => Some(bytes),
            None => return None,
        },
        None => None,
    };
    if literal.is_empty() != key.is_some() {
        return None;
    }
    let same = dictionary
        .and_then(|rows| {
            rows.binary_search_by_key(&row.template, |row| row.template)
                .ok()
                .map(|index| &rows[index])
        })
        .is_some_and(|daily| {
            daily.name == literal
                && (!literal.is_empty()
                    || matches!(&daily.name_key, Recorded::Value(bytes) if Some(bytes.as_slice()) == key))
        });
    Some(if same {
        NameValue::Same
    } else {
        NameValue::Changed {
            literal,
            fallback_key: key,
        }
    })
}
fn span<T>(arena: &[T], range: native::Span) -> Option<&[T]> {
    arena.get(range.start..range.start.checked_add(range.len)?)
}

mod core;
mod countries;
mod dictionary;
mod held;

#[cfg(test)]
mod tests;
