//! Template dictionary, every hour, with exact native text: the counter
//! inputs, the stored 3D-model inputs and the saved model choices of one
//! walk, whose sub-unit definitions resolve through caller-owned slots that
//! grow off-thread, and the names and categories derived from the same walk
//! ([`dictionary`]).
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::allocation::{copy_vec, owned_vec};
use engine::{Memory, creferenced_division_template as native, walk::Outcome};
use replay::{
    frame::{TableInput, TableView},
    rows::{
        DivisionTemplateIconRow, DivisionTemplateRow, Recorded, TableId, TemplateDisplayName,
        TemplateGrids, TemplateNameValue, TemplateSubunit, tables,
    },
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::DivisionTemplates, TableId::DivisionTemplateIcons];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("division template dictionary and country census", 0, &[0]),
    ("division template counter and model inputs", 1, &[1]),
];

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
        let daily = native::walk(memory, base, state, &mut [], &mut [], &mut []);
        let icons = native::walk_icons(
            memory,
            base,
            state,
            &mut [],
            &mut [],
            &mut [],
            &mut [],
            native::DefinitionSlots {
                database: &mut [],
                globals: &mut [],
                locals: &mut [],
            },
        );
        let demand = |outcome| match outcome {
            Outcome::Rows { len } => Ok(len),
            Outcome::Truncated { len, unread_rows } => len
                .checked_add(unread_rows as usize)
                .ok_or("template row demand overflow"),
            Outcome::Unavailable { reason } => Err(reason),
        };
        let rows = demand(daily.outcome)?;
        let icon_rows = demand(icons.outcome)?;
        for (capacity, required) in [
            (&mut self.rows, rows),
            (&mut self.bytes, daily.required_bytes),
            (&mut self.icon_rows, icon_rows),
            (&mut self.icon_bytes, icons.required_bytes),
            (&mut self.name_bytes, daily.required_name_bytes),
            (&mut self.counts, icons.required_counts),
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
pub struct Captured {
    preacquired: Option<Preacquired>,
    rows: CaptureRows<native::Row>,
    bytes: Arena<u8>,
    counts: Arena<native::SubunitCount>,
    variants: Arena<u64>,
    icon_rows: CaptureRows<native::IconRow>,
    icon_bytes: Arena<u8>,
    name_bytes: Arena<u8>,
    sample_hour: Option<i32>,
    /// The dictionary's held name keys, kept across hours.
    keys: dictionary::KeyStore,
    model_failures: ModelFailures,
    database: Arena<usize>,
    globals: Arena<native::GlobalSlot>,
    locals: Arena<native::LocalSlot>,
}
impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.icon_rows
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
        // A failed capture has no rows of its own to count.
        self.model_failures = ModelFailures::default();
        self.keys.forget();
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            bytes: 0,
            counts: 0,
            variants: 0,
            icon_rows: 0,
            icon_bytes: 0,
            name_bytes: 0,
            database: 0,
            locals: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            preacquired: None,
            rows: CaptureRows::new(capacity.rows),
            bytes: Arena::new(capacity.bytes),
            counts: Arena::new(capacity.counts),
            variants: Arena::new(capacity.variants),
            icon_rows: CaptureRows::new(capacity.icon_rows),
            icon_bytes: Arena::new(capacity.icon_bytes),
            name_bytes: Arena::new(capacity.name_bytes),
            sample_hour: None,
            keys: dictionary::KeyStore::default(),
            model_failures: ModelFailures::default(),
            database: Arena::new(capacity.database),
            globals: Arena::new(capacity.database),
            locals: Arena::new(capacity.locals),
        }
    }
    pub fn reset(&mut self) {
        self.preacquired = None;
        self.rows.reset();
        self.bytes.reset();
        self.counts.reset();
        self.variants.reset();
        self.icon_rows.reset();
        self.icon_bytes.reset();
        self.name_bytes.reset();
        self.sample_hour = None;
        self.model_failures = ModelFailures::default();
        self.database.reset();
        self.globals.reset();
        self.locals.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.bytes.allocated_bytes()
            + self.counts.allocated_bytes()
            + self.variants.allocated_bytes()
            + self.icon_rows.allocated_bytes()
            + self.icon_bytes.allocated_bytes()
            + self.name_bytes.allocated_bytes()
            + self.keys.allocated_bytes()
            + self.database.allocated_bytes()
            + self.globals.allocated_bytes()
            + self.locals.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        self.bytes.grow(&mut capacity.bytes);
        self.counts.grow(&mut capacity.counts);
        self.variants.grow(&mut capacity.variants);
        self.icon_rows.grow(&mut capacity.icon_rows);
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
                return marker.ticks.take().unwrap_or([0; N]);
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
        self.capture_original(memory, base, state, hour, force)
    }
    pub(crate) fn capture_preacquired(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force: bool,
    ) {
        let ticks = self.capture_original(memory, base, state, hour, force);
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
    fn capture_original(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        _force: bool,
    ) -> [u64; N] {
        self.reset();
        self.sample_hour = Some(hour);
        // The counter/model inputs, each composition grid's entries and the
        // demanded display-name inputs every hour.
        let icons = {
            let _reads = crate::read_accounting::table(TABLES[1]);
            let start = qpc();
            let walk = native::walk_icons_with_names_growing(
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
            );
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
            if let Some(rows) = self.icon_rows.rows.get(..walk.outcome.len()) {
                for reason in rows.iter().filter_map(|row| row.model_failure) {
                    self.model_failures.add(reason);
                }
            }
            qpc().saturating_sub(start).max(0) as u64
        };
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
            &crate::template_changes::Observer,
            crate::sampled_verify::current_burned_in(crate::template_changes::VERIFY),
            &mut self.rows.rows,
            &mut self.bytes.first,
        );
        self.bytes.len = self.bytes.first.len();
        self.bytes.required = self.bytes.len;
        [qpc().saturating_sub(start).max(0) as u64, icons]
    }
    /// This hour's rows whose model inputs failed, by reason.
    pub(crate) fn model_failures(&self) -> ModelFailures {
        self.model_failures
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome, self.icon_rows.outcome]
    }
    #[cfg(test)]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs().map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("template conversion returns sampled views"),
        })
    }

    /// A display name equal to this frame's complete dictionary row is a
    /// same-frame certificate; a failed or truncated dictionary leaves every
    /// name a self-contained Changed override.
    pub(crate) fn table_inputs(&self) -> [TableInput; N] {
        let dictionary = self.templates();
        let names = match &dictionary {
            TableView::Complete(rows) => rows.rows::<tables::DivisionTemplates>(),
            _ => None,
        };
        let icons = self.icons(names);
        [TableInput::Sampled(dictionary), TableInput::Sampled(icons)]
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
            name_key: replay::rows::Recorded::Value(owned_vec(copy_vec(
                span(bytes, r.name_key).unwrap(),
            ))),
            unit_type: r.unit_type,
        })
    }

    fn icons(&self, dictionary: Option<&[DivisionTemplateRow]>) -> TableView {
        let Some(names) = self.name_bytes.first.get(..self.name_bytes.len) else {
            return unavailable("template display name arena invalid");
        };
        let outcome = self.icon_rows.outcome;
        let Some(bytes) = self.icon_bytes.first.get(..self.icon_bytes.len) else {
            return unavailable("template icon byte arena invalid");
        };
        let Some(counts) = self.counts.first.get(..self.counts.len) else {
            return unavailable("template grouped arena invalid");
        };
        let Some(variants) = self.variants.first.get(..self.variants.len) else {
            return unavailable("template variant arena invalid");
        };
        let Some(rows) = self.icon_rows.rows.get(..outcome.len()) else {
            return unavailable("template icon row length invalid");
        };
        if rows.iter().any(|r| {
            span(bytes, r.priority_unit_name).is_none()
                || span(bytes, r.model_entity).is_none()
                || span(counts, r.counts).is_none()
                || r.grids.iter().any(|grid| span(counts, *grid).is_none())
                || span(variants, r.excluded_variants).is_none()
        }) {
            return unavailable("template icon row spans outside captured arenas");
        }
        let entries = |range: native::Span| {
            owned_vec(engine::allocation::collect_vec(
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
        table::<tables::DivisionTemplateIcons, _>(outcome, &self.icon_rows.rows, |r| {
            let [regiments, support, regimental_support] = r.grids;
            DivisionTemplateIconRow {
                template: r.template,
                priority_unit: r.priority_unit,
                priority_unit_name: owned_vec(copy_vec(span(bytes, r.priority_unit_name).unwrap())),
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
                formation_division_count: replay::rows::Recorded::Value(r.formation_division_count),
                display_name: name_inputs(r, names, dictionary, self.sample_hour),
                grids: Recorded::Value(TemplateGrids {
                    regiments: entries(regiments),
                    support: entries(support),
                    regimental_support: entries(regimental_support),
                }),
            }
        })
    }
}

/// Exact byte comparison, not a digest or pointer identity. Dictionary rows are
/// sorted by the existing table conversion. New handles and changed values need
/// no daily row; overrides make keyframes and direct seeks self-contained.
fn name_inputs(
    row: &native::IconRow,
    bytes: &[u8],
    dictionary: Option<&[DivisionTemplateRow]>,
    hour: Option<i32>,
) -> Recorded<Option<TemplateDisplayName>> {
    if hour.is_none() {
        return Recorded::Value(None);
    }
    let Some(name) = row.display_name else {
        return Recorded::Value(None);
    };
    let Some(literal) = span(bytes, name.literal) else {
        return Recorded::Value(None);
    };
    let key = match name.fallback_key {
        Some(range) => match span(bytes, range) {
            Some(bytes) => Some(bytes),
            None => return Recorded::Value(None),
        },
        None => None,
    };
    if literal.is_empty() != key.is_some() {
        return Recorded::Value(None);
    }
    let same = dictionary.and_then(|rows| {
        rows.binary_search_by_key(&row.template, |row| row.template).ok().map(|index| &rows[index])
    }).is_some_and(|daily| {
        daily.name == literal && (!literal.is_empty()
            || matches!(&daily.name_key, Recorded::Value(bytes) if Some(bytes.as_slice()) == key))
    });
    let value = if same {
        TemplateNameValue::SameAsDivisionTemplates
    } else {
        TemplateNameValue::Changed {
            literal: owned_vec(copy_vec(literal)),
            fallback_key: key.map(|key| owned_vec(copy_vec(key))),
        }
    };
    Recorded::Value(Some(TemplateDisplayName {
        sample_hour: replay::rows::PacketHour::Host,
        value,
    }))
}
fn span<T>(arena: &[T], range: native::Span) -> Option<&[T]> {
    arena.get(range.start..range.start.checked_add(range.len)?)
}

mod dictionary;

#[cfg(test)]
mod tests;
