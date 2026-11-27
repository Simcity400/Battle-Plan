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
    country_rows: Vec<CountryRow>,
}

impl RawClean {
    fn materialize(&self) -> Option<[replay_data::rows::TableRows; 2]> {
        let (dictionary, refused) =
            engine_allocation::contained(|| build_templates(&self.rows, &self.bytes, None));
        let TableInput::Sampled(TableView::Complete(dictionary)) = dictionary else {
            return None;
        };
        if refused {
            return None;
        }
        let (icons, refused) = engine_allocation::contained(|| {
            build_icons(
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
                dictionary.rows::<tables::DivisionTemplates>(),
            )
        });
        let TableInput::Sampled(TableView::Complete(icons)) = icons else {
            return None;
        };
        if refused {
            return None;
        }
        Some([dictionary, icons])
    }
}

const CLEAN_ALLOC_FAILURE: &str = "template clean snapshot allocation failed";

struct LazyClean {
    raw: RawClean,
    country: std::sync::Arc<replay_data::rows::TableRows>,
    tables: std::sync::OnceLock<[replay_data::rows::TableRows; 2]>,
    fallback_reasons: [std::sync::Mutex<Option<String>>; 2],
    fallback_spent: [std::sync::atomic::AtomicBool; 2],
}

impl LazyClean {
    fn tables(&self) -> Option<&[replay_data::rows::TableRows; 2]> {
        if let Some(tables) = self.tables.get() {
            return Some(tables);
        }
        let built = self.raw.materialize()?;
        let _ = self.tables.set(built);
        self.tables.get()
    }

    fn refill_fallback_reason(&self, index: usize) {
        if !self.fallback_spent[index].load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let Some(reason) =
            engine_allocation::advisory(|| engine_allocation::string(CLEAN_ALLOC_FAILURE).ok())
        else {
            return;
        };
        let mut slot = self.fallback_reasons[index]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(reason);
        }
        self.fallback_spent[index].store(false, std::sync::atomic::Ordering::Release);
    }

    fn refill_fallback_reasons(&self) {
        self.refill_fallback_reason(0);
        self.refill_fallback_reason(1);
    }

    fn table_input(&self, index: usize) -> TableInput {
        let (cloned, refused) = engine_allocation::contained(|| {
            self.tables()
                .and_then(|tables| tables[index].try_clone().ok())
        });
        if !refused
            && let Some(rows) = cloned
        {
            self.refill_fallback_reason(index);
            return TableInput::Sampled(TableView::Complete(rows));
        }
        let (reason, reason_refused) =
            engine_allocation::contained(|| engine_allocation::string(CLEAN_ALLOC_FAILURE).ok());
        if !reason_refused
            && let Some(reason) = reason
        {
            return TableView::Unavailable { reason }.into();
        }
        let mut slot = self.fallback_reasons[index]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(reason) = slot.take() {
            self.fallback_spent[index].store(true, std::sync::atomic::Ordering::Release);
            return TableView::Unavailable { reason }.into();
        }
        drop(slot);
        let _ = engine_allocation::oom();
        TableView::Unavailable {
            reason: String::new(),
        }
        .into()
    }
}

const DIRTY_PATCH_MAX_TEMPLATES: usize = 128;
const DIRTY_PATCH_MAX_COUNTRIES: usize = 64;
const DIRTY_PATCH_MAX_DEAD_BYTES: usize = 128 * 1024;

#[derive(Clone)]
struct CleanState {
    tokens: ((u64, u64), (u64, u64)),
    log_cursor: Option<u64>,
    dead_bytes: usize,
    base: usize,
    state: usize,
    digest: native::RosterDigest,
    witness: Witness,
    generation: u64,
    model_failures: ModelFailures,
    required: [usize; 8],
    country_slots: engine_allocation::SharedBox<Vec<countries::CleanCountry>>,
    pointer_index: engine_allocation::SharedBox<Vec<(usize, u32)>>,
    clean: engine_allocation::SharedBox<LazyClean>,
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
    core_serial: u64,
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
    clean_log_cursor: Option<u64>,
    clean_digest: Option<native::RosterDigest>,
    clean_context: Option<(usize, usize)>,
    dirty_patched: bool,
    dirty_dead_bytes: usize,
    dirty_receivers: Vec<usize>,
    dirty_template_indices: Vec<usize>,
    dirty_country_indices: Vec<usize>,
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
        self.clean_log_cursor = None;
        self.clean_digest = None;
        self.clean_context = None;
        self.dirty_patched = false;
        self.dirty_dead_bytes = 0;
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
            clean_log_cursor: None,
            clean_digest: None,
            clean_context: None,
            dirty_patched: false,
            dirty_dead_bytes: 0,
            dirty_receivers: Vec::new(),
            dirty_template_indices: Vec::new(),
            dirty_country_indices: Vec::new(),
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
        self.clean_log_cursor = None;
        self.clean_digest = None;
        self.clean_context = None;
        self.dirty_patched = false;
        self.dirty_dead_bytes = 0;
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
            self.dirty_patched = false;
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
            + (self.dirty_receivers.capacity()
                + self.dirty_template_indices.capacity()
                + self.dirty_country_indices.capacity())
                * size_of::<usize>()
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
        let log_cursor = tokens.0.and_then(|_| {
            let cursor = changes.dirty_cursor()?;
            (changes.quiet() == tokens.0).then_some(cursor)
        });
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
                clean.clean.refill_fallback_reasons();
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
        if !force
            && !changes.fresh_only()
            && let Some(clean) = prev_clean.as_ref()
            && let (Some(q0), Some(q1)) = tokens
            && q1 == clean.tokens.1
            && base == clean.base
            && state == clean.state
            && let Some(cursor) = log_cursor
            && let Some(ticks) = self.try_patch_dirty(
                memory,
                base,
                state,
                hour,
                changes,
                clean,
                (q0, q1),
                cursor,
            )
        {
            if self.fast_clean.is_none() {
                self.prev_clean = prev_clean;
            }
            return ticks;
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
            self.clean_log_cursor = log_cursor.filter(|&cursor| changes.dirty_cursor() == Some(cursor));
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

    #[expect(
        clippy::too_many_arguments,
        reason = "in-place dirty patching takes memory, game state, hour, change source, previous clean snapshot and verified tokens"
    )]
    fn try_patch_dirty(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        changes: &dyn crate::template_changes::Changes,
        clean: &CleanState,
        tokens: ((u64, u64), (u64, u64)),
        log_cursor: u64,
    ) -> Option<[u64; N]> {
        let since = clean.log_cursor?;
        let now_cursor = changes.dirty_receivers_since(since, &mut self.dirty_receivers)?;
        if now_cursor != log_cursor || self.dirty_receivers.is_empty() {
            return None;
        }
        let raw = &clean.clean.raw;
        self.dirty_template_indices.clear();
        self.dirty_country_indices.clear();
        for i in 0..self.dirty_receivers.len() {
            let receiver = self.dirty_receivers[i];
            let start = clean
                .pointer_index
                .partition_point(|&(pointer, _)| pointer < receiver);
            for &(pointer, row_idx) in &clean.pointer_index[start..] {
                if pointer != receiver {
                    break;
                }
                if engine_allocation::advisory(|| {
                    engine_allocation::reserve(&mut self.dirty_template_indices, 1)
                })
                .is_err()
                {
                    return None;
                }
                self.dirty_template_indices.push(row_idx as usize);
            }
            for (c_idx, cc) in clean.country_slots.iter().enumerate() {
                let by_id = cc.id as usize == receiver;
                if by_id || cc.manager == receiver || cc.occupation == Some(receiver) {
                    if engine_allocation::advisory(|| {
                        engine_allocation::reserve(&mut self.dirty_country_indices, 1)
                    })
                    .is_err()
                    {
                        return None;
                    }
                    self.dirty_country_indices.push(c_idx);
                    if by_id {
                        for (row_idx, row) in raw.icon_rows.iter().enumerate() {
                            if row.owner == cc.id {
                                if engine_allocation::advisory(|| {
                                    engine_allocation::reserve(&mut self.dirty_template_indices, 1)
                                })
                                .is_err()
                                {
                                    return None;
                                }
                                self.dirty_template_indices.push(row_idx);
                            }
                        }
                    }
                }
            }
        }
        self.dirty_template_indices.sort_unstable();
        self.dirty_template_indices.dedup();
        self.dirty_country_indices.sort_unstable();
        self.dirty_country_indices.dedup();
        let any_dirty =
            !self.dirty_template_indices.is_empty() || !self.dirty_country_indices.is_empty();
        if any_dirty
            && (clean.model_failures.rows != 0
                || clean.dead_bytes > DIRTY_PATCH_MAX_DEAD_BYTES
                || self.dirty_template_indices.len() > DIRTY_PATCH_MAX_TEMPLATES
                || self.dirty_country_indices.len() > DIRTY_PATCH_MAX_COUNTRIES)
        {
            return None;
        }
        let _reads = collector_memory::read_accounting::table(TABLES[1]);
        let start = qpc();
        if native::clean_roster_digest(memory, base, state) != Ok(clean.digest) {
            return None;
        }
        if !any_dirty {
            if (changes.quiet(), changes.core_dependencies()) != (Some(tokens.0), Some(tokens.1))
                || changes.dirty_cursor() != Some(log_cursor)
            {
                return None;
            }
            let elapsed = qpc().saturating_sub(start).max(0) as u64;
            let icon_count = clean.digest.icon_count;
            let country_count = clean.digest.country_count;
            let mut next = clean.clone();
            next.tokens = tokens;
            next.log_cursor = Some(log_cursor);
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
            next.clean.refill_fallback_reasons();
            self.fast_clean = Some(next);
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
            return Some([0, elapsed, 0]);
        }
        struct StagedTemplate {
            row_idx: usize,
            row: native::IconRow,
            held_cert: held::Certificate,
            core_cert: core::Certificate,
            deployment: usize,
            bytes: Vec<u8>,
            counts: Vec<native::SubunitCount>,
            variants: Vec<u64>,
            names: Vec<u8>,
            speeds: Vec<u32>,
        }
        let mut staged_templates: Vec<StagedTemplate> =
            engine_allocation::advisory(|| engine_allocation::vec(self.dirty_template_indices.len()))
                .ok()?;
        let mut staged_countries: Vec<(usize, countries::Visit)> =
            engine_allocation::advisory(|| engine_allocation::vec(self.dirty_country_indices.len()))
                .ok()?;
        let mut read_rows = Vec::new();
        for &row_idx in &self.dirty_template_indices {
            let old = raw.icon_rows.get(row_idx)?;
            let cc = clean.country_slots.iter().find(|cc| cc.id == old.owner)?;
            if cc.manager == 0 {
                return None;
            }
            let held_cert = [
                changes.sequence(old.pointer)?,
                changes.sequence(old.owner as usize)?,
            ];
            let core_cert = [
                held_cert[0],
                held_cert[1],
                changes.deployment_reset_sequence(cc.manager)?,
            ];
            let mut bytes = Vec::new();
            let mut counts = Vec::new();
            let mut variants = Vec::new();
            let mut names = Vec::new();
            let mut speeds = Vec::new();
            read_rows.clear();
            let walk = engine_allocation::advisory(|| {
                native::read_icon_occurrence(
                    memory,
                    base,
                    state,
                    cc.canonical_slot,
                    old.settings.roster_ordinal,
                    true,
                    &mut read_rows,
                    &mut bytes,
                    &mut counts,
                    &mut variants,
                    &mut names,
                )
            });
            if !matches!(walk.outcome, Outcome::Rows { len: 1 }) {
                return None;
            }
            let value = read_rows[0];
            if value.template != old.template
                || value.pointer != old.pointer
                || value.owner != old.owner
                || value.settings.roster_ordinal != old.settings.roster_ordinal
                || !value.details_fresh
                || value.model_failure.is_some()
                || value.unit_type_failure.is_some()
                || value.equipment_failure.is_some()
                || value.details_failure.is_some()
                || value.display_name.is_none()
                || value.name_key.is_none()
            {
                return None;
            }
            bytes.truncate(walk.bytes_len);
            counts.truncate(walk.counts_len);
            variants.truncate(walk.variants_len);
            names.truncate(walk.name_bytes_len);
            let display = value.display_name?;
            let key_span = value.name_key?;
            span(&names, display.literal)?;
            span(&names, key_span)?;
            if let Some(fallback) = display.fallback_key {
                span(&names, fallback)?;
            }
            if engine_allocation::advisory(|| {
                engine_native::carmy::movement_speed_keys(memory, base, value.pointer, &mut speeds)
            })
            .is_err()
            {
                return None;
            }
            staged_templates.push(StagedTemplate {
                row_idx,
                row: value,
                held_cert,
                core_cert,
                deployment: cc.manager,
                bytes,
                counts,
                variants,
                names,
                speeds,
            });
        }
        let mut country_ticks = 0_u64;
        let mut garrison_scratch = Vec::new();
        for &c_idx in &self.dirty_country_indices {
            let cc = *clean.country_slots.get(c_idx)?;
            let c_start = qpc();
            let _reads = collector_memory::read_accounting::table(TABLES[2]);
            let country = engine_native_memory::country_identity::Country {
                id: cc.id,
                pointer: cc.pointer,
                canonical_slot: cc.canonical_slot,
            };
            let visit = engine_allocation::advisory(|| {
                self.country_cache
                    .observe(memory, base, state, country, changes, &mut garrison_scratch)
            });
            country_ticks += qpc().saturating_sub(c_start).max(0) as u64;
            if visit.garrison_failure().is_some() || !visit.is_ok() {
                return None;
            }
            staged_countries.push((c_idx, visit));
        }
        if (changes.quiet(), changes.core_dependencies()) != (Some(tokens.0), Some(tokens.1))
            || changes.dirty_cursor() != Some(log_cursor)
        {
            return None;
        }
        let extra_icon_bytes: usize = staged_templates.iter().map(|s| s.bytes.len()).sum();
        let extra_counts: usize = staged_templates.iter().map(|s| s.counts.len()).sum();
        let extra_variants: usize = staged_templates.iter().map(|s| s.variants.len()).sum();
        let extra_name_bytes: usize = staged_templates.iter().map(|s| s.names.len()).sum();
        let extra_speeds: usize = staged_templates.iter().map(|s| s.speeds.len()).sum();
        let extra_dict_bytes: usize = staged_templates
            .iter()
            .map(|s| {
                s.row.display_name.unwrap().literal.len + s.row.name_key.unwrap().len
            })
            .sum();
        let prepared = engine_allocation::advisory(|| {
            engine_native_memory::walk::Buffer::Growing(&mut self.rows.rows)
                .ensure(raw.rows.len())
                .ok()?;
            engine_native_memory::walk::Buffer::Growing(&mut self.bytes.first)
                .ensure(raw.bytes.len().checked_add(extra_dict_bytes)?)
                .ok()?;
            engine_native_memory::walk::Buffer::Growing(&mut self.icon_rows.rows)
                .ensure(raw.icon_rows.len())
                .ok()?;
            engine_native_memory::walk::Buffer::Growing(&mut self.icon_bytes.first)
                .ensure(raw.icon_bytes.len().checked_add(extra_icon_bytes)?)
                .ok()?;
            engine_native_memory::walk::Buffer::Growing(&mut self.name_bytes.first)
                .ensure(raw.name_bytes.len().checked_add(extra_name_bytes)?)
                .ok()?;
            engine_native_memory::walk::Buffer::Growing(&mut self.counts.first)
                .ensure(raw.counts.len().checked_add(extra_counts)?)
                .ok()?;
            engine_native_memory::walk::Buffer::Growing(&mut self.variants.first)
                .ensure(raw.variants.len().checked_add(extra_variants)?)
                .ok()?;
            engine_native_memory::walk::Buffer::Growing(&mut self.country_rows.rows)
                .ensure(raw.country_rows.len())
                .ok()?;
            self.payloads.clear();
            engine_allocation::reserve(&mut self.payloads, raw.payloads.len()).ok()?;
            self.speed_keys.clear();
            engine_allocation::reserve(
                &mut self.speed_keys,
                raw.speed_keys.len().checked_add(extra_speeds)?,
            )
            .ok()?;
            self.speed_spans.clear();
            engine_allocation::reserve(&mut self.speed_spans, raw.speed_spans.len()).ok()?;
            Some(())
        });
        prepared?;
        self.rows.rows[..raw.rows.len()].copy_from_slice(&raw.rows);
        self.bytes.first[..raw.bytes.len()].copy_from_slice(&raw.bytes);
        self.bytes.len = raw.bytes.len();
        self.icon_rows.rows[..raw.icon_rows.len()].copy_from_slice(&raw.icon_rows);
        self.icon_bytes.first[..raw.icon_bytes.len()].copy_from_slice(&raw.icon_bytes);
        self.icon_bytes.len = raw.icon_bytes.len();
        self.name_bytes.first[..raw.name_bytes.len()].copy_from_slice(&raw.name_bytes);
        self.name_bytes.len = raw.name_bytes.len();
        self.counts.first[..raw.counts.len()].copy_from_slice(&raw.counts);
        self.counts.len = raw.counts.len();
        self.variants.first[..raw.variants.len()].copy_from_slice(&raw.variants);
        self.variants.len = raw.variants.len();
        for (dst, src) in self.country_rows.rows[..raw.country_rows.len()]
            .iter_mut()
            .zip(&raw.country_rows)
        {
            *dst = src.clone();
        }
        self.payloads.extend(raw.payloads.iter().cloned());
        self.speed_keys.extend_from_slice(&raw.speed_keys);
        self.speed_spans.extend_from_slice(&raw.speed_spans);
        self.core.clear_changed();
        self.held.clear_changed();
        self.country_cache.changed = false;
        let read_count = staged_templates.len() as u64;
        let committed = (|| -> Option<()> {
            for patch in staged_templates {
                let core_serial = self.core.patch_dirty(
                    changes,
                    patch.core_cert,
                    patch.deployment,
                    &patch.row,
                    &patch.bytes,
                    &patch.counts,
                    &patch.variants,
                    &patch.names,
                    &patch.speeds,
                )?;
                if !self.held.patch_dirty(
                    &patch.row,
                    &patch.counts,
                    &patch.variants,
                    &patch.bytes,
                    patch.held_cert,
                ) {
                    return None;
                }
                let offsets = [
                    self.icon_bytes.len,
                    self.counts.len,
                    self.variants.len,
                    self.name_bytes.len,
                ];
                self.icon_bytes.first[offsets[0]..offsets[0] + patch.bytes.len()]
                    .copy_from_slice(&patch.bytes);
                self.icon_bytes.len += patch.bytes.len();
                self.counts.first[offsets[1]..offsets[1] + patch.counts.len()]
                    .copy_from_slice(&patch.counts);
                self.counts.len += patch.counts.len();
                self.variants.first[offsets[2]..offsets[2] + patch.variants.len()]
                    .copy_from_slice(&patch.variants);
                self.variants.len += patch.variants.len();
                self.name_bytes.first[offsets[3]..offsets[3] + patch.names.len()]
                    .copy_from_slice(&patch.names);
                self.name_bytes.len += patch.names.len();
                let shifted = split::shifted(patch.row, offsets, false);
                self.icon_rows.rows[patch.row_idx] = shifted;
                let literal = span(&patch.names, patch.row.display_name.unwrap().literal)?;
                let name_key = span(&patch.names, patch.row.name_key.unwrap())?;
                let lit_start = self.bytes.len;
                self.bytes.first[lit_start..lit_start + literal.len()].copy_from_slice(literal);
                self.bytes.len += literal.len();
                let key_start = self.bytes.len;
                self.bytes.first[key_start..key_start + name_key.len()].copy_from_slice(name_key);
                self.bytes.len += name_key.len();
                self.rows.rows[patch.row_idx] = native::Row {
                    template: patch.row.template,
                    owner: patch.row.owner,
                    name: native::Span {
                        start: lit_start,
                        len: literal.len(),
                    },
                    name_key: native::Span {
                        start: key_start,
                        len: name_key.len(),
                    },
                    unit_type: patch.row.unit_type,
                };
                let speed_start = self.speed_keys.len();
                self.speed_keys.extend_from_slice(&patch.speeds);
                self.speed_spans[patch.row_idx] =
                    (patch.row.template, Some((speed_start, patch.speeds.len())));
                let (layout, layout_serial, settings, settings_serial) =
                    self.held.value_refs(&shifted);
                self.payloads[patch.row_idx] = Payload {
                    key: (shifted.template, shifted.pointer, shifted.owner),
                    core_serial,
                    layout: layout.cloned(),
                    layout_serial,
                    settings: settings.cloned(),
                    settings_serial,
                };
            }
            for (c_idx, visit) in staged_countries {
                let row = self.country_cache.commit_dirty(visit).ok()?;
                self.country_rows.rows[c_idx] = row;
            }
            Some(())
        })();
        if committed.is_none() {
            self.held.forget();
            self.core.forget();
            self.country_cache.forget();
            self.country_snapshot = None;
            self.payloads.clear();
            self.speed_keys.clear();
            self.speed_spans.clear();
            self.bytes.reset();
            self.icon_bytes.reset();
            self.name_bytes.reset();
            self.counts.reset();
            self.variants.reset();
            return None;
        }
        let icon_count = raw.icon_rows.len();
        let country_count = raw.country_rows.len();
        self.sample_hour = Some(hour);
        self.rows.outcome = Outcome::Rows { len: icon_count };
        self.icon_rows.outcome = Outcome::Rows { len: icon_count };
        self.country_rows.outcome = Outcome::Rows { len: country_count };
        self.model_failures = ModelFailures::default();
        self.bytes.required = clean.required[0].max(self.bytes.len);
        self.icon_bytes.required = clean.required[1].max(self.icon_bytes.len);
        self.name_bytes.required = clean.required[2].max(self.name_bytes.len);
        self.counts.required = clean.required[3].max(self.counts.len);
        self.variants.required = clean.required[4].max(self.variants.len);
        self.database.required = clean.required[5];
        self.globals.required = clean.required[6];
        self.locals.required = clean.required[7];
        self.occurrences = [(icon_count as u64).saturating_sub(read_count), read_count];
        let added_bytes = extra_dict_bytes
            + extra_icon_bytes
            + extra_name_bytes
            + extra_counts * size_of::<native::SubunitCount>()
            + extra_variants * size_of::<u64>()
            + extra_speeds * size_of::<u32>();
        self.dirty_dead_bytes = clean.dead_bytes.saturating_add(added_bytes);
        self.dirty_patched = true;
        self.pending_owned = true;
        self.clean_tokens = Some(tokens);
        self.clean_log_cursor = Some(log_cursor);
        self.clean_digest = Some(clean.digest);
        self.clean_context = Some((base, state));
        self.clean = (!self.country_cache.changed).then_some([
            base as u64,
            state as u64,
            icon_count as u64,
            country_count as u64,
            clean.digest.roster,
        ]);
        crate::template_changes::record_countries(country_count);
        crate::template_changes::record(
            &crate::template_changes::Counts {
                carried: (icon_count as u64).saturating_sub(read_count),
                read: read_count,
                rereads: 0,
            },
            Some(icon_count),
        );
        let elapsed = qpc().saturating_sub(start).max(0) as u64;
        Some([0, elapsed.saturating_sub(country_ticks), country_ticks])
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
        let payloads = if self.dirty_patched {
            true
        } else {
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
            let ok = engine_allocation::reserve(&mut self.payloads, rows.len()).is_ok();
            if ok {
                for row in rows {
                    let (layout, layout_serial, settings, settings_serial) =
                        self.held.value_refs(row);
                    self.payloads.push(Payload {
                        key: (row.template, row.pointer, row.owner),
                        core_serial: self.core.core_serial(row),
                        layout: layout.cloned(),
                        layout_serial,
                        settings: settings.cloned(),
                        settings_serial,
                    });
                }
            }
            if matches!(self.country_rows.outcome, Outcome::Rows { .. }) {
                self.country_cache.prune();
            } else {
                self.country_snapshot = None;
            }
            ok
        };
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
            let log_cursor = self.clean_log_cursor.take();
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
            let prev = self.prev_clean.take();
            let reused_indices = prev
                .as_ref()
                .filter(|prev| prev.digest == digest)
                .map(|prev| {
                    (
                        engine_allocation::SharedBox::clone(&prev.country_slots),
                        engine_allocation::SharedBox::clone(&prev.pointer_index),
                    )
                });
            let dead_bytes = if self.dirty_patched {
                self.dirty_dead_bytes
            } else {
                0
            };
            let clean_and_indices = match prev
                .filter(|prev| prev.generation == self.generation && prev.digest == digest)
            {
                Some(prev) => Some((prev.clean, prev.country_slots, prev.pointer_index, prev.dead_bytes)),
                None => engine_allocation::advisory(|| {
                    let country = self.country_snapshot.clone()?;
                    let dict_rows = self.rows.rows.get(..self.rows.outcome.len())?;
                    let dict_bytes = self.bytes.first.get(..self.bytes.len)?;
                    let icon_rows = self.icon_rows.rows.get(..self.icon_rows.outcome.len())?;
                    let icon_bytes = self.icon_bytes.first.get(..self.icon_bytes.len)?;
                    let name_bytes = self.name_bytes.first.get(..self.name_bytes.len)?;
                    let counts = self.counts.first.get(..self.counts.len)?;
                    let variants = self.variants.first.get(..self.variants.len)?;
                    let country_rows_slice = self
                        .country_rows
                        .rows
                        .get(..self.country_rows.outcome.len())?;
                    if !template_spans_valid(dict_rows, dict_bytes)
                        || !icon_spans_valid(icon_rows, icon_bytes, counts, variants)
                    {
                        return None;
                    }
                    let (country_slots, pointer_index) = match reused_indices {
                        Some(indices) => indices,
                        None => {
                            let slots = self.country_cache.clean_countries()?;
                            let mut index = engine_allocation::vec(icon_rows.len()).ok()?;
                            for (at, row) in icon_rows.iter().enumerate() {
                                index.push((row.pointer, u32::try_from(at).ok()?));
                            }
                            index.sort_unstable_by_key(|&(pointer, _)| pointer);
                            (
                                engine_allocation::shared(slots).ok()?,
                                engine_allocation::shared(index).ok()?,
                            )
                        }
                    };
                    let mut payloads = Vec::new();
                    engine_allocation::reserve(&mut payloads, self.payloads.len()).ok()?;
                    payloads.extend(self.payloads.iter().cloned());
                    let mut country_rows = Vec::new();
                    engine_allocation::reserve(&mut country_rows, country_rows_slice.len()).ok()?;
                    country_rows.extend(country_rows_slice.iter().cloned());
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
                        country_rows,
                    };
                    let fallback_reasons = [
                        std::sync::Mutex::new(Some(
                            engine_allocation::string(CLEAN_ALLOC_FAILURE).ok()?,
                        )),
                        std::sync::Mutex::new(Some(
                            engine_allocation::string(CLEAN_ALLOC_FAILURE).ok()?,
                        )),
                    ];
                    Some((
                        engine_allocation::shared(LazyClean {
                            raw,
                            country,
                            tables: std::sync::OnceLock::new(),
                            fallback_reasons,
                            fallback_spent: [
                                std::sync::atomic::AtomicBool::new(false),
                                std::sync::atomic::AtomicBool::new(false),
                            ],
                        })
                        .ok()?,
                        country_slots,
                        pointer_index,
                        dead_bytes,
                    ))
                }),
            };
            clean_and_indices.map(|(clean, country_slots, pointer_index, dead_bytes)| CleanState {
                tokens: clean_tokens,
                log_cursor,
                dead_bytes,
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
                country_slots,
                pointer_index,
                clean,
            })
        } else {
            self.clean_log_cursor = None;
            self.prev_clean = None;
            None
        };
        self.dirty_patched = false;
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
                Some(fast) => fast.clean.table_input(0),
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
                Some(fast) => fast.clean.table_input(1),
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
    let row_index = |row: &native::IconRow| -> Option<usize> {
        let ptr = std::ptr::from_ref(row);
        rows.as_ptr_range().contains(&ptr).then(|| {
            (ptr as usize - rows.as_ptr() as usize) / size_of::<native::IconRow>()
        })
    };
    let find_payload = |at: Option<usize>, row: &native::IconRow| -> Option<&Payload> {
        let key = (row.template, row.pointer, row.owner);
        at.and_then(|at| payloads.get(at).filter(|payload| payload.key == key))
            .or_else(|| payloads.iter().find(|payload| payload.key == key))
    };
    let payload_refs = |row: &native::IconRow| {
        find_payload(row_index(row), row).map_or((None, None), |payload| {
            (payload.layout.as_ref(), payload.settings.as_ref())
        })
    };
    let payload_serials = |at: usize, row: &native::IconRow| {
        find_payload(Some(at), row).map_or((0, 0, 0), |payload| {
            (
                payload.core_serial,
                payload.layout_serial,
                payload.settings_serial,
            )
        })
    };
    let speed_key_slice_at = |at: Option<usize>, template: u64| {
        let entry = at
            .and_then(|at| speed_spans.get(at).filter(|entry| entry.0 == template))
            .or_else(|| speed_spans.iter().find(|entry| entry.0 == template))?;
        let (start, len) = entry.1?;
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
    const CORE_MASK: u64 = (1 << 0)
        | (1 << 1)
        | (1 << 2)
        | (1 << 3)
        | (1 << 4)
        | (1 << 5)
        | (1 << 6)
        | (1 << 7)
        | (1 << 8)
        | (1 << 9)
        | (1 << 10)
        | (1 << 11)
        | (1 << 12)
        | (1 << 13)
        | (1 << 15)
        | (1 << 16)
        | (1 << 17)
        | (1 << 20);
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct IconFixed {
        core_serial: u64,
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
            if prev.core_serial != 0 && cur.core_serial != 0 {
                if prev.core_serial == cur.core_serial {
                    mask |= CORE_MASK;
                }
                if prev.name_state == cur.name_state && (cur.name_state != 2 || same_bytes) {
                    mask |= 1 << 14;
                }
            } else if prev.core_serial == 0 && cur.core_serial == 0 {
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
            } else if prev.name_state == cur.name_state && cur.name_state != 2 {
                mask |= 1 << 14;
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
        |at, r, record| {
            let (core_serial, layout_serial, settings_serial) = payload_serials(at, r);
            if core_serial != 0 {
                let name_state = match name_value_at(Some(at), r, names, dictionary, sample_hour) {
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
                return Some(IconFixed {
                    core_serial,
                    priority_unit: 0,
                    custom_icon: 0,
                    custom_icon_nato: 0,
                    representative: 0,
                    representative_local: 0,
                    country: 0,
                    foreign_country: 0,
                    model_category: None,
                    model_unreadable: 0,
                    formation_division_count: None,
                    manpower_exile: false,
                    base_width: None,
                    name_state,
                    layout_serial,
                    settings_serial,
                });
            }
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
            let name_state = match name_value_at(Some(at), r, names, dictionary, sample_hour) {
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
            record.ids(speed_key_slice_at(Some(at), r.template));
            Some(IconFixed {
                core_serial: 0,
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
            let at = row_index(r);
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
                    name_inputs(at, r, names, dictionary, sample_hour)
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
                        speed_key_slice_at(at, r.template).map(|keys| owned_vec(copy_vec(keys))),
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
    at: Option<usize>,
    row: &native::IconRow,
    bytes: &[u8],
    dictionary: Option<&[DivisionTemplateRow]>,
    hour: Option<i32>,
) -> Recorded<Option<TemplateDisplayName>> {
    Recorded::Value(
        name_value_at(at, row, bytes, dictionary, hour).map(|value| TemplateDisplayName {
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

fn name_value_at<'a>(
    at: Option<usize>,
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
            at.and_then(|at| rows.get(at).filter(|daily| daily.template == row.template))
                .or_else(|| {
                    rows.binary_search_by_key(&row.template, |row| row.template)
                        .ok()
                        .map(|index| &rows[index])
                })
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
