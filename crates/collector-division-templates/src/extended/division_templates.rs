use collector_primitives::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
};
use collector_projection::projection::{
    Projection, Witness, held_input, ledger_table_masked, project_table,
};
use collector_projection::table_conversion::{table, unavailable};
use engine_allocation::{copy_vec, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::creferenced_division_template as native;
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

pub const MODEL_REASONS: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelFailures {
    pub rows: u32,
    pub reasons: [(&'static str, u32); MODEL_REASONS],
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
    database: usize,
    locals: usize,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    bytes,
    counts,
    variants,
    icon_rows,
    country_rows,
    icon_bytes,
    name_bytes,
    database,
    locals,
});

impl Capacities {
    /// # Safety
    /// Run only during attach or the active session's serial sizing pass against the authenticated
    /// session image, while template and subunit observer storage remains live and teardown is
    /// excluded.
    pub unsafe fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let countries = engine_native::ccountry::Reader::new(memory, base, state)?
            .count()
            .saturating_sub(1) as usize;
        collector_primitives::table_capture::grow_capacity(&mut self.country_rows, countries);
        let daily = native::count(memory, base, state);
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
        crate::template_changes::prepare(icon_rows.as_ref().copied().unwrap_or(0), countries);
        collector_model_inputs::local_subunit_changes::prepare(countries);
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
            collector_primitives::table_capture::grow_capacity(capacity, required);
        }
        Ok(())
    }
}

pub use split::{TemplateWorkers, WalkPart};

struct Preacquired {
    base: usize,
    state: usize,
    hour: i32,
    force: bool,
    ticks: Option<[u64; N]>,
}
#[derive(Default, Clone, PartialEq)]
struct CountryRow {
    garrison_priority: Option<i32>,
    state: engine_deployment::cdeployment_status::TemplateState,
    garrisons: Option<std::sync::Arc<Vec<GarrisonTemplateSelection>>>,
    garrison_failure: Option<&'static str>,
}
struct RawClean {
    rows: Vec<native::Row>,
    bytes: Vec<u8>,
    icon_rows: Vec<native::IconRow>,
    icon_bytes: Vec<u8>,
    name_bytes: Vec<u8>,
    counts: Vec<native::SubunitCount>,
    variants: Vec<u64>,
    sample_hour: Option<i32>,
    payloads: Vec<Payload>,
    speed_keys: Vec<u32>,
    speed_spans: Vec<(u64, Option<(usize, usize)>)>,
}

impl RawClean {
    fn materialize(&self) -> Option<[std::sync::Arc<replay_data::rows::TableRows>; 2]> {
        engine_allocation::advisory(|| {
            let dictionary = match build_templates(&self.rows, &self.bytes, None) {
                TableInput::Sampled(TableView::Complete(rows)) => Some(std::sync::Arc::new(rows)),
                _ => None,
            }?;
            let names = dictionary.rows::<tables::DivisionTemplates>();
            let icons = match build_icons(
                &self.icon_rows,
                &self.icon_bytes,
                &self.name_bytes,
                &self.counts,
                &self.variants,
                self.sample_hour,
                &self.payloads,
                &self.speed_keys,
                &self.speed_spans,
                None,
                names,
            ) {
                TableInput::Sampled(TableView::Complete(rows)) => Some(std::sync::Arc::new(rows)),
                _ => None,
            }?;
            Some([dictionary, icons])
        })
    }
}

struct LazyClean {
    raw: RawClean,
    country: std::sync::Arc<replay_data::rows::TableRows>,
    tables: std::sync::OnceLock<Option<[std::sync::Arc<replay_data::rows::TableRows>; 2]>>,
}

impl LazyClean {
    fn tables(&self) -> Option<&[std::sync::Arc<replay_data::rows::TableRows>; 2]> {
        self.tables.get_or_init(|| self.raw.materialize()).as_ref()
    }
}

#[derive(Clone)]
struct CleanState {
    tokens: ((u64, u64), (u64, u64)),
    base: usize,
    state: usize,
    digest: native::RosterDigest,
    witness: Witness,
    generation: u64,
    model_failures: ModelFailures,
    required: [usize; 8],
    clean: std::sync::Arc<LazyClean>,
}

#[derive(Default)]
pub struct Stores {
    held: held::Store,
    core: core::Store,
    countries: countries::Store,
    snapshot: Option<std::sync::Arc<replay_data::rows::TableRows>>,
    generation: u64,
    clean: Option<CleanState>,
}

#[derive(Default)]
pub struct Shelf {
    stores: Option<Stores>,
    bytes: usize,
}

pub type Home = engine_allocation::SharedBox<std::sync::Mutex<Shelf>>;

pub fn new_home() -> std::io::Result<Home> {
    engine_allocation::shared(std::sync::Mutex::default())
}

pub fn home_allocated_bytes(home: &Home) -> usize {
    home.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .bytes
}

#[derive(Clone)]
struct Payload {
    key: (u64, usize, u32),
    layout: Option<std::sync::Arc<TemplateLayout>>,
    layout_serial: u64,
    settings: Option<std::sync::Arc<TemplateSettings>>,
    settings_serial: u64,
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
    held: held::Store,
    core: core::Store,
    home: Option<Home>,
    payloads: Vec<Payload>,
    model_failures: ModelFailures,
    database: Arena<usize>,
    globals: Arena<native::GlobalSlot>,
    locals: Arena<native::LocalSlot>,
    speed_keys: Vec<u32>,
    speed_spans: Vec<(u64, Option<(usize, usize)>)>,
    clean: Option<[u64; 5]>,
    clean_tokens: Option<((u64, u64), (u64, u64))>,
    clean_digest: Option<native::RosterDigest>,
    clean_context: Option<(usize, usize)>,
    prev_clean: Option<CleanState>,
    fast_clean: Option<CleanState>,
    generation: u64,
    witness: Option<Witness>,
    occurrences: [u64; 2],
    walk_parts: Vec<WalkPart>,
    staged_parts: usize,
    split_fallback: Option<collector_workers::hour_stages::TemplateFallback>,
    lent: Option<engine_allocation::SharedBox<split::Lent>>,
    slot_weights: Vec<(u32, u64, u32)>,
    stitching: split::Stitching,
}
impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.clean = None;
        self.clean_tokens = None;
        self.clean_digest = None;
        self.clean_context = None;
        self.prev_clean = None;
        self.fast_clean = None;
        self.witness = None;
        self.pending_owned = false;
        self.rows
            .fail(collector_extended_support::outcomes::failed_capture(
                TABLES[0], hour, force_full, reason,
            ));
        self.icon_rows
            .fail(collector_extended_support::outcomes::failed_capture(
                TABLES[1], hour, force_full, reason,
            ));
        self.model_failures = ModelFailures::default();
        self.held.forget();
        self.core.forget();
        self.country_cache.forget();
        self.country_snapshot = None;
        self.country_rows.fail(Outcome::Unavailable { reason });
    }

    pub fn vacant() -> Self {
        Self::without_home(&Capacities {
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            home: Some(new_home().unwrap()),
            ..Self::without_home(capacity)
        }
    }

    pub fn without_home(capacity: &Capacities) -> Self {
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
            held: held::Store::default(),
            core: core::Store::default(),
            home: None,
            payloads: Vec::new(),
            model_failures: ModelFailures::default(),
            database: Arena::new(capacity.database),
            globals: Arena::new(capacity.database),
            locals: Arena::new(capacity.locals),
            speed_keys: Vec::new(),
            speed_spans: Vec::new(),
            clean: None,
            clean_tokens: None,
            clean_digest: None,
            clean_context: None,
            prev_clean: None,
            fast_clean: None,
            generation: 0,
            witness: None,
            occurrences: [0; 2],
            walk_parts: Vec::new(),
            staged_parts: 0,
            split_fallback: None,
            lent: None,
            slot_weights: Vec::new(),
            stitching: split::Stitching::default(),
        }
    }
    pub fn reset(&mut self) {
        self.discard_owned();
        self.clean = None;
        self.clean_tokens = None;
        self.clean_digest = None;
        self.clean_context = None;
        self.prev_clean = None;
        self.fast_clean = None;
        self.witness = None;
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
        self.staged_parts = 0;
        self.split_fallback = None;
    }
    fn discard_owned(&mut self) {
        if self.pending_owned {
            self.held.forget();
            self.core.forget();
            self.country_cache.forget();
            self.country_snapshot = None;
            self.prev_clean = None;
            self.fast_clean = None;
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
            + self.icon_bytes.allocated_bytes()
            + self.name_bytes.allocated_bytes()
            + self.payloads.capacity() * size_of::<Payload>()
            + self.database.allocated_bytes()
            + self.globals.allocated_bytes()
            + self.locals.allocated_bytes()
            + self.speed_keys.capacity() * size_of::<u32>()
            + self.speed_spans.capacity() * size_of::<(u64, Option<(usize, usize)>)>()
            + self.walk_parts.capacity() * size_of::<WalkPart>()
            + self
                .walk_parts
                .iter()
                .map(WalkPart::allocated_bytes)
                .sum::<usize>()
            + self
                .lent
                .as_ref()
                .map_or(0, |_| engine_allocation::SharedBox::<split::Lent>::BYTES)
            + self.slot_weights.capacity() * size_of::<(u32, u64, u32)>()
            + self.stitching.allocated_bytes()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn home(&self) -> Option<Home> {
        self.home.clone()
    }

    pub fn share_home(&mut self, home: &Home) {
        self.home = Some(home.clone());
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

    pub fn naming_demand(&self) -> usize {
        match self.icon_rows.outcome {
            Outcome::Rows { len } => len,
            Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
            Outcome::Unavailable { .. } => 0,
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// so global `template_changes` sizing demand updates belong to this pass.
    pub unsafe fn capture(
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
        // SAFETY: Caller holds hourly capture admission for this template capture pass.
        unsafe {
            self.capture_original(
                memory,
                base,
                state,
                hour,
                force,
                &crate::template_changes::Observer,
            )
        }
    }
    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// so global `template_changes` sizing demand updates belong to this pass.
    pub unsafe fn capture_preacquired(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force: bool,
        workers: Option<TemplateWorkers<'_>>,
    ) {
        let ticks = self.capture_observed(
            memory,
            base,
            state,
            hour,
            force,
            &crate::template_changes::Observer,
            workers,
        );
        if let Some(fallback) = self.split_fallback {
            collector_workers::hour_stages::note_template_fallback(fallback);
        }
        self.preacquired = Some(Preacquired {
            base,
            state,
            hour,
            force,
            ticks: Some(ticks),
        });
    }
    pub fn fail_preacquired(
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
    pub fn take_preacquired(
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
    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// so global `template_changes` sizing demand updates belong to this pass.
    #[cfg(any(test, feature = "test-support"))]
    pub unsafe fn capture_original(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force: bool,
        changes: &dyn crate::template_changes::Changes,
    ) -> [u64; N] {
        let mut ticks = self.capture_observed(memory, base, state, hour, force, changes, None);
        let start = qpc();
        self.settle_owned();
        ticks[1] += qpc().saturating_sub(start).max(0) as u64;
        ticks
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the hourly capture operands (memory, base, game state, hour, full flag) travel beside the change source and workers"
    )]
    fn capture_observed(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force: bool,
        changes: &dyn crate::template_changes::Changes,
        workers: Option<TemplateWorkers<'_>>,
    ) -> [u64; N] {
        use collector_workers::hour_stages::{Part, record_part};
        self.reset();
        let stores = self
            .home
            .as_ref()
            .and_then(|home| {
                home.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .stores
                    .take()
            })
            .unwrap_or_default();
        self.held = stores.held;
        self.core = stores.core;
        self.country_cache = stores.countries;
        self.country_snapshot = stores.snapshot;
        self.generation = stores.generation;
        let prev_clean = if force { None } else { stores.clean };
        if force {
            self.held.forget();
            self.core.forget();
            self.country_cache.forget();
            self.country_snapshot = None;
        }
        let tokens = (changes.quiet(), changes.core_dependencies());
        if !force
            && !changes.fresh_only()
            && let Some(clean) = prev_clean.as_ref()
            && tokens == (Some(clean.tokens.0), Some(clean.tokens.1))
            && base == clean.base
            && state == clean.state
        {
            let _reads = collector_memory::read_accounting::table(TABLES[1]);
            let start = qpc();
            if native::clean_roster_digest(memory, base, state) == Ok(clean.digest)
                && (changes.quiet(), changes.core_dependencies()) == tokens
            {
                let elapsed = qpc().saturating_sub(start).max(0) as u64;
                let icon_count = clean.digest.icon_count;
                let country_count = clean.digest.country_count;
                self.sample_hour = Some(hour);
                self.rows.outcome = Outcome::Rows { len: icon_count };
                self.icon_rows.outcome = Outcome::Rows { len: icon_count };
                self.country_rows.outcome = Outcome::Rows { len: country_count };
                self.model_failures = clean.model_failures;
                self.occurrences = [icon_count as u64, 0];
                self.bytes.required = clean.required[0];
                self.icon_bytes.required = clean.required[1];
                self.name_bytes.required = clean.required[2];
                self.counts.required = clean.required[3];
                self.variants.required = clean.required[4];
                self.database.required = clean.required[5];
                self.globals.required = clean.required[6];
                self.locals.required = clean.required[7];
                self.witness = Some(clean.witness);
                self.generation = clean.generation;
                self.fast_clean = prev_clean;
                self.pending_owned = true;
                crate::template_changes::record_countries(country_count);
                crate::template_changes::record(
                    &crate::template_changes::Counts {
                        carried: icon_count as u64,
                        read: 0,
                        rereads: 0,
                    },
                    Some(icon_count),
                );
                return [0, elapsed, 0];
            }
        }
        self.prev_clean = prev_clean;
        self.country_cache.begin();
        self.held.begin();
        self.core.begin();
        self.sample_hour = Some(hour);
        let templates: [u64; 2];
        let speed_refusals: u64;
        let country_refusals: u32;
        let country_ticks: u64;
        let icons = {
            let _reads = collector_memory::read_accounting::table(TABLES[1]);
            let start = qpc();
            let walked = workers
                .map(|mut workers| self.walk_split(memory, base, state, changes, &mut workers));
            let split::Walked {
                walk,
                counts,
                country_len,
                country_failure,
                country_ticks: ticks,
            } = match walked {
                Some(Ok(walked)) => walked,
                refused => {
                    if let Some(Err(fallback)) = refused {
                        self.split_fallback = Some(fallback);
                    }
                    let serial = qpc();
                    let walked = self.walk_serial(memory, base, state, changes);
                    record_part(Part::TemplateSerial, serial, qpc());
                    walked
                }
            };
            country_ticks = ticks;
            templates = counts;
            self.occurrences = counts;
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
            let speed = qpc();
            speed_refusals = self.capture_speed_keys(memory, base, changes);
            let certify = qpc();
            record_part(Part::TemplateSpeedKeys, speed, certify);
            country_refusals = self.model_failures.rows;
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
            let certified = qpc();
            record_part(Part::TemplateCertify, certify, certified);
            certified.saturating_sub(start).max(0) as u64
        };
        let country_native_ticks = country_ticks;
        self.country_cache.finish();
        self.pending_owned = true;
        let _reads = collector_memory::read_accounting::table(TABLES[0]);
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
        let (outcome, rereads) = dictionary::derive(
            memory,
            icon_rows,
            self.icon_rows.outcome,
            names,
            &mut self.rows.rows,
            &mut self.bytes.first,
        );
        self.rows.outcome = outcome;
        record_part(Part::TemplateDictionary, start, qpc());
        crate::template_changes::record(
            &crate::template_changes::Counts {
                carried: templates[0],
                read: templates[1],
                rereads,
            },
            matches!(outcome, Outcome::Rows { .. }).then_some(icon_rows.len()),
        );
        self.bytes.len = self.bytes.first.len();
        self.bytes.required = self.bytes.len;
        let complete = self
            .outcomes()
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Rows { .. }));
        let stable = !force
            && complete
            && rereads == 0
            && speed_refusals == 0
            && self.model_failures.rows == country_refusals
            && matches!(tokens, (Some(_), Some(_)))
            && (changes.quiet(), changes.core_dependencies()) == tokens;
        let roster = stable.then(|| {
            icon_rows.iter().fold(native::FNV_OFFSET, |hash, row| {
                native::hash_template(
                    hash,
                    row.template,
                    row.pointer,
                    row.owner,
                    row.settings.roster_ordinal,
                )
            })
        });
        if let Some(roster) = roster
            && !changes.fresh_only()
            && let (Some(q0), Some(q1)) = tokens
            && let Some(countries) = self.country_cache.digest()
        {
            self.clean_tokens = Some((q0, q1));
            self.clean_digest = Some(native::RosterDigest {
                roster,
                countries,
                icon_count: self.icon_rows.outcome.len(),
                country_count: self.country_rows.outcome.len(),
            });
            self.clean_context = Some((base, state));
        }
        let clean = stable && !self.country_cache.changed && !self.held.reordered();
        self.clean = clean.then(|| {
            [
                base as u64,
                state as u64,
                self.icon_rows.outcome.len() as u64,
                self.country_rows.outcome.len() as u64,
                roster.unwrap(),
            ]
        });
        [
            qpc().saturating_sub(start).max(0) as u64,
            icons.saturating_sub(country_native_ticks),
            country_ticks,
        ]
    }

    fn walk_serial(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        changes: &dyn crate::template_changes::Changes,
    ) -> split::Walked {
        let mut country_ticks = 0_u64;
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
            let _reads = collector_memory::read_accounting::table(TABLES[2]);
            let result = (|| {
                let row = country_cache.read(memory, base, state, country, changes)?;
                if let Some(reason) = row.garrison_failure {
                    context_failures.add(reason);
                }
                engine_native_memory::walk::Buffer::Growing(&mut *country_rows)
                    .ensure(country_len + 1)?;
                country_rows[country_len] = row;
                country_len += 1;
                Ok::<(), &'static str>(())
            })();
            country_ticks += qpc().saturating_sub(start).max(0) as u64;
            country_failure = result.err();
        };
        let mut details = held::Demand::new(&mut self.held, &mut self.core, changes);
        let walk = native::walk_icons_with_selected_details(
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
        let counts = details.counts;
        split::Walked {
            walk,
            counts,
            country_len,
            country_failure,
            country_ticks,
        }
    }

    pub fn settle_owned(&mut self) {
        if !self.pending_owned {
            return;
        }
        if let Some(fast) = self.fast_clean.clone() {
            self.pending_owned = false;
            if let Some(home) = &self.home {
                let bytes = self.held.allocated_bytes()
                    + self.core.allocated_bytes()
                    + self.country_cache.allocated_bytes();
                let mut shelf = home.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                shelf.bytes = shelf.bytes.max(bytes);
                shelf.stores = Some(Stores {
                    held: std::mem::take(&mut self.held),
                    core: std::mem::take(&mut self.core),
                    countries: std::mem::take(&mut self.country_cache),
                    snapshot: self.country_snapshot.clone(),
                    generation: self.generation,
                    clean: Some(fast),
                });
            }
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
        self.payloads.clear();
        let payloads = engine_allocation::reserve(&mut self.payloads, rows.len()).is_ok();
        if payloads {
            for row in rows {
                let (layout, layout_serial, settings, settings_serial) = self.held.value_refs(row);
                self.payloads.push(Payload {
                    key: (row.template, row.pointer, row.owner),
                    layout: layout.cloned(),
                    layout_serial,
                    settings: settings.cloned(),
                    settings_serial,
                });
            }
            self.payloads.sort_unstable_by_key(|payload| payload.key);
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
        let failed = collector_memory::terminal::failed() || !payloads;
        if failed {
            self.discard_owned();
            if !payloads {
                let hour = self.sample_hour.unwrap_or_default();
                self.fail_capture(hour, false, "template payload allocation failed");
            }
        } else {
            self.pending_owned = false;
        }
        let clean = self.clean.take().filter(|_| !failed);
        if clean.is_none() || self.core.changed() || self.held.changed() || self.generation == 0 {
            self.generation = collector_projection::projection::generation();
        }
        self.witness = clean.map(|[base, state, icons, countries, roster]| {
            Witness::new([self.generation, base, state, icons, countries, roster])
        });
        let next_clean = if !failed
            && let (Some(clean_tokens), Some(digest), Some((base, state))) = (
                self.clean_tokens.take(),
                self.clean_digest.take(),
                self.clean_context.take(),
            ) {
            let witness = self.witness.unwrap_or_else(|| {
                Witness::new([
                    self.generation,
                    base as u64,
                    state as u64,
                    digest.icon_count as u64,
                    digest.country_count as u64,
                    digest.roster,
                ])
            });
            let clean = match self
                .prev_clean
                .take()
                .filter(|prev| prev.generation == self.generation && prev.digest == digest)
            {
                Some(prev) => Some(prev.clean),
                None => engine_allocation::advisory(|| {
                    let country = self.country_snapshot.clone()?;
                    let dict_rows = self.rows.rows.get(..self.rows.outcome.len())?;
                    let dict_bytes = self.bytes.first.get(..self.bytes.len)?;
                    let icon_rows = self.icon_rows.rows.get(..self.icon_rows.outcome.len())?;
                    let icon_bytes = self.icon_bytes.first.get(..self.icon_bytes.len)?;
                    let name_bytes = self.name_bytes.first.get(..self.name_bytes.len)?;
                    let counts = self.counts.first.get(..self.counts.len)?;
                    let variants = self.variants.first.get(..self.variants.len)?;
                    if !template_spans_valid(dict_rows, dict_bytes)
                        || !icon_spans_valid(icon_rows, icon_bytes, counts, variants)
                    {
                        return None;
                    }
                    let mut payloads = Vec::new();
                    engine_allocation::reserve(&mut payloads, self.payloads.len()).ok()?;
                    payloads.extend(self.payloads.iter().cloned());
                    let raw = RawClean {
                        rows: copy_vec(dict_rows).ok()?,
                        bytes: copy_vec(dict_bytes).ok()?,
                        icon_rows: copy_vec(icon_rows).ok()?,
                        icon_bytes: copy_vec(icon_bytes).ok()?,
                        name_bytes: copy_vec(name_bytes).ok()?,
                        counts: copy_vec(counts).ok()?,
                        variants: copy_vec(variants).ok()?,
                        sample_hour: self.sample_hour,
                        payloads,
                        speed_keys: copy_vec(&self.speed_keys).ok()?,
                        speed_spans: copy_vec(&self.speed_spans).ok()?,
                    };
                    Some(std::sync::Arc::new(LazyClean {
                        raw,
                        country,
                        tables: std::sync::OnceLock::new(),
                    }))
                }),
            };
            clean.map(|clean| CleanState {
                tokens: clean_tokens,
                base,
                state,
                digest,
                witness,
                generation: self.generation,
                model_failures: self.model_failures,
                required: [
                    self.bytes.required,
                    self.icon_bytes.required,
                    self.name_bytes.required,
                    self.counts.required,
                    self.variants.required,
                    self.database.required,
                    self.globals.required,
                    self.locals.required,
                ],
                clean,
            })
        } else {
            self.prev_clean = None;
            None
        };
        if !failed && let Some(home) = &self.home {
            let bytes = self.held.allocated_bytes()
                + self.core.allocated_bytes()
                + self.country_cache.allocated_bytes();
            let mut shelf = home.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            shelf.bytes = shelf.bytes.max(bytes);
            shelf.stores = Some(Stores {
                held: std::mem::take(&mut self.held),
                core: std::mem::take(&mut self.core),
                countries: std::mem::take(&mut self.country_cache),
                snapshot: self.country_snapshot.clone(),
                generation: self.generation,
                clean: next_clean,
            });
        }
    }

    fn capture_speed_keys(
        &mut self,
        memory: &impl Memory,
        base: usize,
        changes: &dyn crate::template_changes::Changes,
    ) -> u64 {
        self.speed_keys.clear();
        self.speed_spans.clear();
        let rows = self
            .icon_rows
            .rows
            .get(..self.icon_rows.outcome.len())
            .unwrap_or(&[]);
        let mut refused = 0_u64;
        if engine_allocation::reserve(&mut self.speed_spans, rows.len()).is_err() {
            refused = rows.len() as u64;
        } else {
            let parts = &self.walk_parts[..self.staged_parts.min(self.walk_parts.len())];
            let mut staged = parts.iter().flat_map(|part| part.staged_speed_keys());
            let mut next = staged.next();
            for (at, row) in rows.iter().enumerate() {
                let start = self.speed_keys.len();
                let span = if let Some(keys) = self.core.speed(row.template) {
                    let span = engine_allocation::reserve(&mut self.speed_keys, keys.len())
                        .ok()
                        .map(|()| {
                            self.speed_keys.extend_from_slice(keys);
                            (start, keys.len())
                        });
                    refused += u64::from(span.is_none());
                    span
                } else {
                    while next.is_some_and(|(staged, _)| staged < at) {
                        next = staged.next();
                    }
                    let read = next.filter(|&(staged, _)| staged == at).map(|(_, keys)| keys);
                    let span = match read {
                        Some(keys) => keys.and_then(|keys| {
                            engine_allocation::reserve(&mut self.speed_keys, keys.len())
                                .ok()
                                .map(|()| {
                                    self.speed_keys.extend_from_slice(keys);
                                    (start, keys.len())
                                })
                        }),
                        None => engine_native::carmy::movement_speed_keys(
                            memory,
                            base,
                            row.pointer,
                            &mut self.speed_keys,
                        )
                        .ok()
                        .map(|()| (start, self.speed_keys.len() - start)),
                    };
                    refused += u64::from(span.is_none());
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
        refused
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn occurrences(&self) -> [u64; 2] {
        self.occurrences
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn split_fallback(&self) -> Option<collector_workers::hour_stages::TemplateFallback> {
        self.split_fallback
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn speed_spans(&self) -> (&[u32], &[(u64, Option<(usize, usize)>)]) {
        (&self.speed_keys, &self.speed_spans)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn model_failures(&self) -> ModelFailures {
        self.model_failures
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.rows.outcome,
            self.icon_rows.outcome,
            self.country_rows.outcome,
        ]
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn tables(&self) -> [TableView; 2] {
        let [dictionary, icons, _] = self.table_inputs();
        [dictionary, icons].map(|input| match input {
            TableInput::Sampled(view) => view,
            TableInput::SharedComplete(rows) => TableView::Complete(rows.try_clone().unwrap()),
            _ => unreachable!("template conversion returns sampled views"),
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn table_inputs(&self) -> [TableInput; N] {
        self.projected_inputs(None)
    }

    pub fn projected_inputs(&self, mut projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
        let witness = self.witness;
        let previous = projection
            .as_deref()
            .and_then(|projection| projection.observed_baseline::<tables::DivisionTemplates>());
        let dictionary = held_input::<tables::DivisionTemplates>(
            projection.as_deref_mut(),
            witness,
            |projection| match &self.fast_clean {
                Some(fast) => match fast.clean.tables() {
                    Some(tables) => TableInput::SharedComplete(std::sync::Arc::clone(&tables[0])),
                    None => unavailable("template clean snapshot allocation failed").into(),
                },
                None => self.templates(projection),
            },
        );
        let complete;
        let names = match &dictionary {
            TableInput::Sampled(TableView::Complete(rows)) => {
                rows.rows::<tables::DivisionTemplates>()
            }
            TableInput::SharedComplete(rows) => rows.rows::<tables::DivisionTemplates>(),
            TableInput::ComparedEqual(_) => previous,
            TableInput::ComparedChanges { .. } => {
                complete = self.templates(None);
                match &complete {
                    TableInput::Sampled(TableView::Complete(rows)) => {
                        rows.rows::<tables::DivisionTemplates>()
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        let icons = held_input::<tables::DivisionTemplateIcons>(
            projection.as_deref_mut(),
            witness,
            |projection| match &self.fast_clean {
                Some(fast) => match fast.clean.tables() {
                    Some(tables) => TableInput::SharedComplete(std::sync::Arc::clone(&tables[1])),
                    None => unavailable("template clean snapshot allocation failed").into(),
                },
                None => self.icons(projection, names),
            },
        );
        let country = held_input::<tables::CountryTemplateState>(projection, witness, |_| {
            if let Some(fast) = &self.fast_clean {
                TableInput::SharedComplete(std::sync::Arc::clone(&fast.clean.country))
            } else if matches!(self.country_rows.outcome, Outcome::Rows { .. }) {
                self.country_snapshot
                    .as_ref()
                    .map(|rows| TableInput::SharedComplete(std::sync::Arc::clone(rows)))
                    .unwrap_or_else(|| TableInput::Sampled(self.country_state()))
            } else {
                TableInput::Sampled(self.country_state())
            }
        });
        [dictionary, icons, country]
    }

    pub fn country_state(&self) -> TableView {
        if let Some(fast) = &self.fast_clean {
            return match fast.clean.country.try_clone() {
                Ok(rows) => TableView::Complete(rows),
                Err(_) => unavailable("country template snapshot clone failed"),
            };
        }
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

    fn templates(&self, projection: Option<&mut Projection<'_>>) -> TableInput {
        let outcome = self.rows.outcome;
        let Some(bytes) = self.bytes.first.get(..self.bytes.len) else {
            return unavailable("template byte arena invalid").into();
        };
        let Some(rows) = self.rows.rows.get(..outcome.len()) else {
            return unavailable("template row length invalid").into();
        };
        if !template_spans_valid(rows, bytes) {
            return unavailable("template row spans outside captured arenas").into();
        }
        project_table::<tables::DivisionTemplates, _>(
            projection,
            outcome,
            &self.rows.rows,
            |r| u128::from(r.template),
            |r, previous| {
                r.owner == previous.owner
                    && r.unit_type == previous.unit_type
                    && span(bytes, r.name).unwrap() == previous.name
                    && match &previous.name_key {
                        Recorded::Value(key) => span(bytes, r.name_key).unwrap() == key,
                        Recorded::NotRecorded => false,
                    }
            },
            |r| DivisionTemplateRow {
                template: r.template,
                owner: r.owner,
                name: owned_vec(copy_vec(span(bytes, r.name).unwrap())),
                name_key: replay_data::rows::Recorded::Value(owned_vec(copy_vec(
                    span(bytes, r.name_key).unwrap(),
                ))),
                unit_type: r.unit_type,
            },
        )
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
        if !icon_spans_valid(rows, bytes, counts, variants) {
            return unavailable("template icon row spans outside captured arenas").into();
        }
        build_icons_with_outcome(
            outcome,
            &self.icon_rows.rows,
            bytes,
            names,
            counts,
            variants,
            self.sample_hour,
            &self.payloads,
            &self.speed_keys,
            &self.speed_spans,
            projection,
            dictionary,
        )
    }
}

fn template_spans_valid(rows: &[native::Row], bytes: &[u8]) -> bool {
    !rows
        .iter()
        .any(|r| span(bytes, r.name).is_none() || span(bytes, r.name_key).is_none())
}

fn icon_spans_valid(
    rows: &[native::IconRow],
    bytes: &[u8],
    counts: &[native::SubunitCount],
    variants: &[u64],
) -> bool {
    !rows.iter().any(|r| {
        span(bytes, r.priority_unit_name).is_none()
            || span(bytes, r.model_entity).is_none()
            || span(counts, r.counts).is_none()
            || r.grids.iter().any(|grid| span(counts, *grid).is_none())
            || r.cells.iter().any(|grid| span(counts, *grid).is_none())
            || span(variants, r.excluded_variants).is_none()
            || span(variants, r.forbidden_equipment).is_none()
    })
}

fn build_templates(
    rows: &[native::Row],
    bytes: &[u8],
    projection: Option<&mut Projection<'_>>,
) -> TableInput {
    project_table::<tables::DivisionTemplates, _>(
        projection,
        Outcome::Rows { len: rows.len() },
        rows,
        |r| u128::from(r.template),
        |r, previous| {
            r.owner == previous.owner
                && r.unit_type == previous.unit_type
                && span(bytes, r.name).unwrap() == previous.name
                && match &previous.name_key {
                    Recorded::Value(key) => span(bytes, r.name_key).unwrap() == key,
                    Recorded::NotRecorded => false,
                }
        },
        |r| DivisionTemplateRow {
            template: r.template,
            owner: r.owner,
            name: owned_vec(copy_vec(span(bytes, r.name).unwrap())),
            name_key: replay_data::rows::Recorded::Value(owned_vec(copy_vec(
                span(bytes, r.name_key).unwrap(),
            ))),
            unit_type: r.unit_type,
        },
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "shared icon row builder takes flat arenas for both live and lazy clean snapshots"
)]
fn build_icons(
    rows: &[native::IconRow],
    bytes: &[u8],
    names: &[u8],
    counts: &[native::SubunitCount],
    variants: &[u64],
    sample_hour: Option<i32>,
    payloads: &[Payload],
    speed_keys: &[u32],
    speed_spans: &[(u64, Option<(usize, usize)>)],
    projection: Option<&mut Projection<'_>>,
    dictionary: Option<&[DivisionTemplateRow]>,
) -> TableInput {
    build_icons_with_outcome(
        Outcome::Rows { len: rows.len() },
        rows,
        bytes,
        names,
        counts,
        variants,
        sample_hour,
        payloads,
        speed_keys,
        speed_spans,
        projection,
        dictionary,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "shared icon row builder takes flat arenas for both live and lazy clean snapshots"
)]
fn build_icons_with_outcome(
    outcome: Outcome,
    rows: &[native::IconRow],
    bytes: &[u8],
    names: &[u8],
    counts: &[native::SubunitCount],
    variants: &[u64],
    sample_hour: Option<i32>,
    payloads: &[Payload],
    speed_keys: &[u32],
    speed_spans: &[(u64, Option<(usize, usize)>)],
    projection: Option<&mut Projection<'_>>,
    dictionary: Option<&[DivisionTemplateRow]>,
) -> TableInput {
    let payload_refs = |row: &native::IconRow| {
        payloads
            .binary_search_by_key(&(row.template, row.pointer, row.owner), |payload| {
                payload.key
            })
            .map_or((None, None), |at| {
                let payload = &payloads[at];
                (payload.layout.as_ref(), payload.settings.as_ref())
            })
    };
    let payload_serials = |row: &native::IconRow| {
        payloads
            .binary_search_by_key(&(row.template, row.pointer, row.owner), |payload| {
                payload.key
            })
            .map_or((0, 0), |at| {
                let payload = &payloads[at];
                (payload.layout_serial, payload.settings_serial)
            })
    };
    let speed_key_slice = |template: u64| {
        let at = speed_spans
            .binary_search_by_key(&template, |entry| entry.0)
            .ok()?;
        let (start, len) = speed_spans[at].1?;
        speed_keys.get(start..start.checked_add(len)?)
    };
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
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct IconFixed {
        priority_unit: u32,
        custom_icon: i32,
        custom_icon_nato: u8,
        representative: u32,
        representative_local: u8,
        country: i32,
        foreign_country: i32,
        model_category: Option<u64>,
        model_unreadable: u8,
        formation_division_count: Option<i32>,
        manpower_exile: bool,
        base_width: Option<i64>,
        name_state: u8,
        layout_serial: u64,
        settings_serial: u64,
    }
    impl IconFixed {
        fn keep_mask(prev: &Self, cur: &Self, same_bytes: bool) -> u64 {
            let mut mask = 0_u64;
            if prev.priority_unit == cur.priority_unit {
                mask |= 1 << 0;
            }
            if same_bytes {
                mask |= (1 << 1) | (1 << 6) | (1 << 10) | (1 << 11) | (1 << 15) | (1 << 20);
            }
            if prev.custom_icon == cur.custom_icon {
                mask |= 1 << 2;
            }
            if prev.custom_icon_nato == cur.custom_icon_nato {
                mask |= 1 << 3;
            }
            if prev.representative == cur.representative {
                mask |= 1 << 4;
            }
            if prev.representative_local == cur.representative_local {
                mask |= 1 << 5;
            }
            if prev.country == cur.country {
                mask |= 1 << 7;
            }
            if prev.foreign_country == cur.foreign_country {
                mask |= 1 << 8;
            }
            if prev.model_category == cur.model_category {
                mask |= 1 << 9;
            }
            if prev.model_unreadable == cur.model_unreadable {
                mask |= 1 << 12;
            }
            if prev.formation_division_count == cur.formation_division_count {
                mask |= 1 << 13;
            }
            if prev.name_state == cur.name_state && (cur.name_state != 2 || same_bytes) {
                mask |= 1 << 14;
            }
            if prev.manpower_exile == cur.manpower_exile {
                mask |= 1 << 16;
            }
            if prev.base_width == cur.base_width {
                mask |= 1 << 17;
            }
            if prev.layout_serial == cur.layout_serial {
                mask |= 1 << 18;
            }
            if prev.settings_serial == cur.settings_serial {
                mask |= 1 << 19;
            }
            mask
        }
    }
    let record_entries =
        |range: native::Span, record: &mut collector_projection::projection::Record<'_>| {
            let slice = span(counts, range).unwrap();
            record.word(slice.len() as i64);
            for unit in slice {
                record.word(i64::from(unit.token) | (i64::from(u8::from(unit.local)) << 32));
                record.word(i64::from(unit.count));
            }
        };
    ledger_table_masked::<tables::DivisionTemplateIcons, _, IconFixed>(
        projection,
        outcome,
        rows,
        |r| u128::from(r.template),
        |_, r, record| {
            record.bytes(Some(span(bytes, r.priority_unit_name).unwrap()));
            record.bytes(Some(span(bytes, r.model_entity).unwrap()));
            let excluded = span(variants, r.excluded_variants).unwrap();
            record.word(excluded.len() as i64);
            for &variant in excluded {
                record.word(variant as i64);
            }
            record_entries(r.counts, record);
            for grid in r.grids {
                record_entries(grid, record);
            }
            let name_state = match name_value(r, names, dictionary, sample_hour) {
                None => 0,
                Some(NameValue::Same) => 1,
                Some(NameValue::Changed {
                    literal,
                    fallback_key,
                }) => {
                    record.bytes(Some(literal));
                    record.bytes(fallback_key);
                    2
                }
            };
            record.ids(speed_key_slice(r.template));
            let (layout_serial, settings_serial) = payload_serials(r);
            Some(IconFixed {
                priority_unit: r.priority_unit,
                custom_icon: r.custom_icon,
                custom_icon_nato: r.custom_icon_nato,
                representative: r.representative.token,
                representative_local: u8::from(r.representative.local),
                country: r.country,
                foreign_country: r.foreign_country,
                model_category: r.model_category,
                model_unreadable: u8::from(r.model_failure.is_some()),
                formation_division_count: r.formation_division_count,
                manpower_exile: r.manpower_exile,
                base_width: base_width(r),
                name_state,
                layout_serial,
                settings_serial,
            })
        },
        IconFixed::keep_mask,
        |r, kept| {
            let [regiments, support, regimental_support] = r.grids;
            let (layout, settings) = if kept & ((1 << 18) | (1 << 19)) == ((1 << 18) | (1 << 19)) {
                (Recorded::Value(None), Recorded::Value(None))
            } else {
                let (layout, settings) = payload_refs(r);
                (
                    Recorded::Value(if kept & (1 << 18) != 0 {
                        None
                    } else {
                        layout.cloned()
                    }),
                    Recorded::Value(if kept & (1 << 19) != 0 {
                        None
                    } else {
                        settings.cloned()
                    }),
                )
            };
            DivisionTemplateIconRow {
                manpower_exile: Recorded::Value(r.manpower_exile),
                template: r.template,
                base_width: Recorded::Value(if kept & (1 << 17) != 0 {
                    None
                } else {
                    base_width(r)
                }),
                priority_unit: r.priority_unit,
                priority_unit_name: if kept & (1 << 1) != 0 {
                    Vec::new()
                } else {
                    owned_vec(copy_vec(span(bytes, r.priority_unit_name).unwrap()))
                },
                custom_icon: r.custom_icon,
                custom_icon_nato: r.custom_icon_nato,
                representative: r.representative.token,
                representative_local: u8::from(r.representative.local),
                subunits: if kept & (1 << 6) != 0 {
                    Vec::new()
                } else {
                    entries(r.counts)
                },
                country: r.country,
                foreign_country: r.foreign_country,
                model_category: r.model_category,
                model_entity: if kept & (1 << 10) != 0 {
                    Vec::new()
                } else {
                    owned_vec(copy_vec(span(bytes, r.model_entity).unwrap()))
                },
                excluded_variants: if kept & (1 << 11) != 0 {
                    Vec::new()
                } else {
                    owned_vec(copy_vec(span(variants, r.excluded_variants).unwrap()))
                },
                model_unreadable: u8::from(r.model_failure.is_some()),
                formation_division_count: replay_data::rows::Recorded::Value(
                    r.formation_division_count,
                ),
                display_name: if kept & (1 << 14) != 0 {
                    Recorded::Value(None)
                } else {
                    name_inputs(r, names, dictionary, sample_hour)
                },
                grids: if kept & (1 << 15) != 0 {
                    Recorded::Value(TemplateGrids::default())
                } else {
                    Recorded::Value(TemplateGrids {
                        regiments: entries(regiments),
                        support: entries(support),
                        regimental_support: entries(regimental_support),
                    })
                },
                layout,
                settings,
                speed_keys: if kept & (1 << 20) != 0 {
                    Recorded::Value(None)
                } else {
                    Recorded::Value(
                        speed_key_slice(r.template).map(|keys| owned_vec(copy_vec(keys))),
                    )
                },
            }
        },
    )
}

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

fn name_value<'a>(
    row: &native::IconRow,
    bytes: &'a [u8],
    dictionary: Option<&[DivisionTemplateRow]>,
    hour: Option<i32>,
) -> Option<NameValue<'a>> {
    hour?;
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
mod split;

#[cfg(any(test, feature = "test-support"))]
pub mod tests;

#[cfg(test)]
#[path = "division_templates/projection_tests.rs"]
mod projection_tests;
