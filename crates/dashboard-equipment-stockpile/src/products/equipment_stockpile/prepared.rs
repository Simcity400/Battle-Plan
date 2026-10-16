//! The shared stockpile frame step feeding integer runs and dated supplying reads.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use dashboard_base::products::equipment::VariantVersion;

const FIELDS: [&str; 8] = [
    "stock",
    "daily",
    "daily-ic",
    "cost-low",
    "cost-high",
    "balance",
    "logistics-stock",
    "available",
];
const COVERAGE: [&str; 4] = ["stock-hours", "line-hours", "balance-hours", "ai-hours"];
pub mod choices;

/// Numeric values and row presence are independent: no row inside known coverage
/// is zero, whereas an explicit uncomputable row remains unknown.
#[derive(Default, Serialize, Deserialize)]
struct Numbers {
    keys: BTreeMap<String, u64>,
    amounts: [Accumulator; 8],
    present: [Accumulator; 8],
    coverage: Accumulator,
    reads: Accumulator,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Holder {
    variants: BTreeSet<u64>,
    logistics: BTreeSet<u32>,
    stock_read: bool,
    line_read: bool,
    balance_read: bool,
    ai_read: bool,
    costless: usize,
    other_kinds: usize,
    missing_available: usize,
    overflowed: [usize; 8],
    tokens: BTreeMap<u32, String>,
    nonzero_logistics: BTreeSet<u32>,
    groups: BTreeMap<String, i32>,
}

/// Current supplying-read evidence, rebuilt with the native step after restore.
/// Equal reads change dates and diagnostic counts, not the numeric projection.
#[derive(Default)]
struct RepeatedReads {
    stock: bool,
    lines: bool,
    logistics: Option<bool>,
    stock_overflow: usize,
    line_counts: [usize; 6],
    logistics_counts: [usize; 4],
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: BTreeMap<String, ScanState>,
    holders: BTreeMap<String, Holder>,
    numbers: Numbers,
    names: VariantCatalogue,
    naming: Accumulator,
    refusals: catalogue::Refusals,
    choices: choices::Choices,
    designs: BTreeMap<u64, BTreeSet<String>>,
    first_design: BTreeMap<u64, (i32, String)>,
    active: BTreeSet<String>,
    frontier: Option<i32>,
    naming_ready: bool,
    #[serde(skip)]
    country_reads: Option<dashboard_base::products::territory::CountryReads>,
    #[serde(skip)]
    repeated: BTreeMap<String, RepeatedReads>,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}

impl Prepared {
    #[cfg(test)]
    pub fn observe(
        &mut self,
        format: replay::format::FormatVersion,
        frame: &DecodedFrame,
    ) -> io::Result<()> {
        self.observe_changed(
            format,
            frame,
            &FrameChanges {
                rows_changed: [true; replay::frame::TABLE_COUNT],
                table_read: [true; replay::frame::TABLE_COUNT],
                slot_read: [true; replay::frame::TABLE_COUNT],
            },
        )
    }
    pub fn observe_changed(
        &mut self,
        format: replay::format::FormatVersion,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.naming.advance(frame.hour)?;
        self.numbers.advance(frame.hour)?;
        self.choices.advance(frame.hour)?;
        let continuous = self.frontier.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        self.frontier = Some(frame.hour);
        let countries_changed = if changes.rows_changed[TableId::Countries.index()]
            || changes.rows_changed[TableId::CountryLinks.index()]
            || !continuous
        {
            let rows = frame.complete::<tables::Countries>().ok();
            let same = rows.is_some_and(|rows| {
                self.country_reads
                    .as_ref()
                    .is_some_and(|seen| seen.matches(frame, rows))
            });
            if !same {
                self.country_reads = rows.map(|rows| {
                    if !changes.rows_changed[TableId::CountryLinks.index()]
                        && let Some(reads) = &self.country_reads
                    {
                        dashboard_base::products::territory::CountryReads::from_slots(
                            frame,
                            rows,
                            Arc::clone(reads.slots()),
                        )
                    } else {
                        dashboard_base::products::territory::CountryReads::new(frame, rows)
                    }
                });
            }
            !same
        } else {
            false
        };
        let naming_changed = !continuous
            || countries_changed
            || VariantCatalogue::TABLES
                .iter()
                .filter(|&&table| !matches!(table, TableId::Countries | TableId::CountryLinks))
                .any(|table| changes.rows_changed[table.index()])
            || (!self.naming_ready && changes.slot_read[TableId::VariantDefinitions.index()]);
        let numeric_changed = !continuous
            || countries_changed
            || [
                TableId::Stockpiles,
                TableId::ProductionLines,
                TableId::LogisticsEquipment,
            ]
            .iter()
            .any(|table| changes.rows_changed[table.index()]);
        let stock_read = changes.slot_read[TableId::Stockpiles.index()];
        let logistics_read = changes.slot_read[TableId::LogisticsEquipment.index()];
        let repeat_ready = self.active.iter().all(|tag| {
            self.repeated.get(tag).is_some_and(|read| {
                (!stock_read
                    || (read.stock || frame.complete::<tables::Stockpiles>().is_err())
                        && (read.lines || frame.complete::<tables::ProductionLines>().is_err()))
                    && (!logistics_read
                        || read.logistics.is_some()
                        || frame.complete::<tables::LogisticsEquipment>().is_err())
            })
        });
        if !naming_changed && !numeric_changed && repeat_ready {
            self.names.advance(frame.hour);
            for tag in &self.active {
                let state = self.scan.get_mut(tag).expect("active stockpile country");
                state.advance(frame.hour);
                let scanned = &mut state.result;
                let repeat = &self.repeated[tag];
                let holder = self.holders.get_mut(tag).expect("current stockpile holder");
                if stock_read {
                    if repeat.stock {
                        scanned.stock.overflowed += repeat.stock_overflow;
                        holder.overflowed[0] = scanned.stock.overflowed;
                        scanned.reads.clear();
                        scanned.reads.push(frame.hour);
                        self.numbers.read(tag, 0, frame.hour)?;
                    }
                    if repeat.lines {
                        scanned.costless += repeat.line_counts[0];
                        scanned.other_kinds += repeat.line_counts[1];
                        holder.costless = scanned.costless;
                        holder.other_kinds = scanned.other_kinds;
                        for (index, rows) in [
                            &mut scanned.daily,
                            &mut scanned.daily_ic,
                            &mut scanned.cost_low,
                            &mut scanned.cost_high,
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            rows.overflowed += repeat.line_counts[index + 2];
                            holder.overflowed[index + 1] = rows.overflowed;
                        }
                        scanned.line_reads.clear();
                        scanned.line_reads.push(frame.hour);
                        self.numbers.read(tag, 1, frame.hour)?;
                    }
                }
                if logistics_read && repeat.logistics == Some(true) {
                    scanned.missing_available += repeat.logistics_counts[0];
                    holder.missing_available = scanned.missing_available;
                    for (index, rows) in [
                        &mut scanned.balance,
                        &mut scanned.logistics_stock,
                        &mut scanned.available,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        rows.overflowed += repeat.logistics_counts[index + 1];
                        holder.overflowed[index + 5] = rows.overflowed;
                    }
                    scanned.logistics_reads.clear();
                    scanned.logistics_reads.push(frame.hour);
                    self.numbers.read(tag, 2, frame.hour)?;
                }
            }
            return Ok(());
        }
        let dirty_names = if naming_changed {
            // Input changes can precede their supplying read, including after
            // restore. Retain that invalidation until the catalogue can read it;
            // the supplying-read flags still come from this accepted frame.
            let mut naming_changes = FrameChanges {
                rows_changed: changes.rows_changed,
                table_read: changes.table_read,
                slot_read: changes.slot_read,
            };
            naming_changes.rows_changed[TableId::VariantDefinitions.index()] |= !self.naming_ready;
            let dirty: std::collections::BTreeSet<u64> = self
                .names
                .observe_changed(format, frame, &naming_changes)
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            self.naming_ready = changes.slot_read[TableId::VariantDefinitions.index()]
                && [
                    TableId::Countries,
                    TableId::CountryLinks,
                    TableId::VariantDefinitions,
                    TableId::LexicalTokens,
                ]
                .iter()
                .all(|table| {
                    matches!(
                        &*frame.tables[table.index()],
                        replay::frame::TableView::Complete(_)
                    )
                });
            self.choices.classify(&self.names, &dirty, frame.hour)?;
            dirty
        } else {
            self.names.advance(frame.hour);
            BTreeSet::new()
        };
        if naming_changed {
            for id in dirty_names {
                let versions = self
                    .names
                    .versions
                    .get_mut(&id)
                    .expect("changed variant naming");
                #[cfg(test)]
                {
                    self.work.history_visits += 1;
                }
                let written = self
                    .naming
                    .state(id)
                    .and_then(|state| state.last)
                    .map(|run| run.value as i32);
                for version in versions
                    .iter()
                    .filter(|version| written.is_none_or(|hour| version.first_hour > hour))
                {
                    self.naming.event(id, version.first_hour, version)?;
                    if let Some(raw) = &version.raw {
                        if let Some(design) = &raw.design {
                            self.designs.entry(id).or_default().insert(design.clone());
                            self.first_design
                                .entry(id)
                                .or_insert_with(|| (version.first_hour, design.clone()));
                        }
                        for (_, why) in &raw.refusals {
                            self.refusals.note(why, id.to_string());
                        }
                    }
                }
                if let Some(version) = versions
                    .last()
                    .filter(|version| written.is_none_or(|hour| version.first_hour > hour))
                {
                    self.naming.observe(
                        id,
                        frame.hour,
                        Some(i128::from(version.first_hour)),
                        false,
                    );
                }
                // The final unknown marker and the most recent nonempty token are
                // enough for observe/current/token; dated versions live in events.
                let known = versions.iter().rposition(|version| {
                    version
                        .raw
                        .as_ref()
                        .is_some_and(|raw| !raw.token.is_empty())
                });
                let last = versions.len().saturating_sub(1);
                let mut index = 0;
                versions.retain(|_| {
                    let keep = index == last || Some(index) == known;
                    index += 1;
                    keep
                });
            }
        }
        self.naming.through(frame.hour);
        #[cfg(test)]
        let rows_before = ROW_VISITS.with(std::cell::Cell::get);
        let input = FrameRows::resolved(
            frame,
            self.country_reads.as_ref(),
            changes.slot_read[TableId::Stockpiles.index()],
            changes.slot_read[TableId::LogisticsEquipment.index()],
        );
        #[cfg(test)]
        {
            self.work.row_visits += ROW_VISITS.with(std::cell::Cell::get) - rows_before;
        }
        for tag in input.identities.keys() {
            self.scan.entry(tag.clone()).or_default();
        }
        let touched: BTreeSet<_> = input
            .identities
            .keys()
            .cloned()
            .chain(self.active.iter().cloned())
            .collect();
        for tag in &touched {
            let state = self.scan.get_mut(tag).expect("observed stockpile country");
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            let before = read_counts(&state.result);
            state.observe(format, &input, input.identities.get(tag).copied());
            let repeat = self.repeated.entry(tag.clone()).or_default();
            if !input.identities.contains_key(tag) {
                *repeat = RepeatedReads::default();
            } else {
                let after = read_counts(&state.result);
                if stock_read {
                    repeat.stock = input.stock.is_some();
                    repeat.lines = input.lines.is_some();
                    repeat.stock_overflow = after[0] - before[0];
                    for (index, count) in repeat.line_counts.iter_mut().enumerate() {
                        *count = after[index + 1] - before[index + 1];
                    }
                }
                if logistics_read {
                    repeat.logistics = input
                        .logistics
                        .as_ref()
                        .map(|rows| !rows.get(&input.identities[tag].0).is_none_or(Vec::is_empty));
                    for (index, count) in repeat.logistics_counts.iter_mut().enumerate() {
                        *count = after[index + 7] - before[index + 7];
                    }
                }
            }
            let scanned = &mut state.result;
            let holder = self.holders.entry(tag.clone()).or_default();
            let new_variants: BTreeSet<_> = scanned
                .stock
                .runs
                .keys()
                .chain(scanned.daily.runs.keys())
                .copied()
                .filter(|id| !holder.variants.contains(id))
                .collect();
            let new_logistics: BTreeSet<_> = scanned
                .balance
                .runs
                .keys()
                .chain(scanned.logistics_stock.runs.keys())
                .copied()
                .filter(|id| !holder.logistics.contains(id))
                .collect();
            holder.variants.extend(
                scanned
                    .stock
                    .runs
                    .keys()
                    .chain(scanned.daily.runs.keys())
                    .copied(),
            );
            holder.logistics.extend(
                scanned
                    .balance
                    .runs
                    .keys()
                    .chain(scanned.logistics_stock.runs.keys())
                    .copied(),
            );
            holder.stock_read |= !scanned.reads.is_empty();
            holder.line_read |= !scanned.line_reads.is_empty();
            holder.balance_read |= !scanned.logistics_reads.is_empty();
            holder.ai_read |= !scanned.ai.spans.is_empty();
            holder.costless = scanned.costless;
            holder.other_kinds = scanned.other_kinds;
            holder.missing_available = scanned.missing_available;
            for token in scanned
                .balance
                .runs
                .keys()
                .chain(scanned.logistics_stock.runs.keys())
            {
                if let Some(name) = scanned.logistics_tokens.get(token) {
                    holder.tokens.entry(*token).or_insert_with(|| name.clone());
                }
            }
            for (&token, runs) in scanned
                .balance
                .runs
                .iter()
                .chain(scanned.logistics_stock.runs.iter())
                .chain(scanned.available.runs.iter())
            {
                if runs.last().is_some_and(|run| {
                    run.last_hour == frame.hour && run.amount_raw.as_deref() != Some("0")
                }) {
                    holder.nonzero_logistics.insert(token);
                }
            }
            self.choices.observe(
                tag,
                holder,
                scanned,
                &self.names,
                frame.hour,
                &mut self.numbers,
            )?;
            holder.overflowed = [
                scanned.stock.overflowed,
                scanned.daily.overflowed,
                scanned.daily_ic.overflowed,
                scanned.cost_low.overflowed,
                scanned.cost_high.overflowed,
                scanned.balance.overflowed,
                scanned.logistics_stock.overflowed,
                scanned.available.overflowed,
            ];
            self.numbers.observe(
                tag,
                frame.hour,
                scanned,
                holder,
                &new_variants,
                &new_logistics,
            )?;
        }
        self.repeated
            .retain(|tag, _| input.identities.contains_key(tag));
        self.active = input.identities.keys().cloned().collect();
        Ok(())
    }

    pub fn bytes(&self) -> usize {
        self.numbers.bytes() + self.naming.bytes() + self.choices.bytes()
    }

    pub fn content(
        source: &Source,
        content: Option<&dashboard_core::config::ContentOptions>,
    ) -> choices::Content {
        choices::Content::load(source, content)
    }
    pub fn bind_content(&mut self, content: &choices::Content) {
        self.choices.bind_content(content);
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        for tag in &self.active {
            self.scan
                .get_mut(tag)
                .expect("active stockpile country")
                .seal();
        }
        self.numbers.flush(writer, branch, cancelled)?;
        self.choices.flush(writer, branch, cancelled)?;
        self.naming
            .flush(writer, &format!("stockpile-{branch}-names"), 0, cancelled)?;
        series::write_catalogue(
            writer,
            "stockpile",
            branch,
            &self.numbers.keys,
            &(
                &self.holders,
                &self.names,
                self.refusals
                    .diagnostic_lines("equipment variants with unknown text/design operands"),
                &self.designs,
                &self.first_design,
                self.choices.metadata(),
            ),
        )
    }
}

fn read_counts(scanned: &ScannedStockpile) -> [usize; 11] {
    [
        scanned.stock.overflowed,
        scanned.costless,
        scanned.other_kinds,
        scanned.daily.overflowed,
        scanned.daily_ic.overflowed,
        scanned.cost_low.overflowed,
        scanned.cost_high.overflowed,
        scanned.missing_available,
        scanned.balance.overflowed,
        scanned.logistics_stock.overflowed,
        scanned.available.overflowed,
    ]
}

impl Numbers {
    fn read(&mut self, tag: &str, field: usize, hour: i32) -> io::Result<()> {
        let key = self.key(tag, &format!("reads-{field}"));
        self.reads.observe(key, hour, Some(i128::from(hour)), false);
        self.reads.event(key, hour, &hour)
    }
    fn advance(&mut self, hour: i32) -> io::Result<()> {
        for series in self
            .amounts
            .iter_mut()
            .chain(self.present.iter_mut())
            .chain([&mut self.coverage, &mut self.reads])
        {
            series.advance(hour)?;
        }
        Ok(())
    }
    fn key(&mut self, tag: &str, field: &str) -> u64 {
        let next = self.keys.len() as u64;
        *self
            .keys
            .entry(series::field_key(tag, field))
            .or_insert(next)
    }

    fn values<K: Ord + Copy + std::fmt::Display>(
        &mut self,
        tag: &str,
        field: usize,
        hour: i32,
        rows: &mut ReadRuns<K>,
        ids: &BTreeSet<K>,
        added: &BTreeSet<K>,
    ) {
        let span = rows.spans.last().filter(|span| span.last_hour == hour);
        let starts = span.is_some_and(|span| span.first_hour == hour);
        let changed: BTreeSet<_> = if span.is_none() || starts {
            ids.clone()
        } else {
            rows.runs
                .keys()
                .copied()
                .chain(added.iter().copied())
                .collect()
        };
        for id in &changed {
            let key = self.key(tag, &format!("{}:{id}", FIELDS[field]));
            let Some(run) = rows
                .runs
                .get(id)
                .and_then(|runs| runs.last())
                .filter(|run| run.last_hour == hour)
            else {
                // Known absence is independently retained from an explicit
                // uncomputable row, and contributes zero to the numeric sum.
                observe_numeric(
                    &mut self.present[field],
                    key,
                    hour,
                    span.map(|_| 0),
                    starts,
                    false,
                );
                observe_numeric(
                    &mut self.amounts[field],
                    key,
                    hour,
                    span.map(|_| 0),
                    starts,
                    false,
                );
                continue;
            };
            // A new supplying interval must remain separate even when the status
            // or number matches the last interval before an identity/table break.
            let boundary = run.first_hour == hour;
            let value = run
                .amount_raw
                .as_ref()
                .map(|value| value.parse::<i128>().expect("ReadRuns i64"));
            let previous = self.present[field].state(key).and_then(|state| state.last);
            let version = if boundary {
                previous.map_or(1, |run| run.value.abs() + 1)
            } else {
                previous.expect("carried stockpile presence").value.abs()
            };
            observe_numeric(
                &mut self.present[field],
                key,
                hour,
                Some(if value.is_some() { version } else { -version }),
                starts,
                false,
            );
            observe_numeric(&mut self.amounts[field], key, hour, value, starts, boundary);
        }
        // ReadRuns retains its own open/carry state. Closed history is already in
        // Accumulator; retaining one current interval is sufficient for the step.
        rows.runs.retain(|_, runs| {
            if runs.last().is_none_or(|run| run.last_hour != hour) {
                return false;
            }
            runs.drain(..runs.len().saturating_sub(1));
            true
        });
        rows.spans.drain(..rows.spans.len().saturating_sub(1));
    }

    fn observe(
        &mut self,
        tag: &str,
        hour: i32,
        scanned: &mut ScannedStockpile,
        holder: &Holder,
        new_variants: &BTreeSet<u64>,
        new_logistics: &BTreeSet<u32>,
    ) -> io::Result<()> {
        for (field, spans) in [
            &scanned.stock.spans,
            &scanned.daily.spans,
            &scanned.balance.spans,
            &scanned.ai.spans,
        ]
        .into_iter()
        .enumerate()
        {
            let key = self.key(tag, COVERAGE[field]);
            let span = spans.last().filter(|span| span.last_hour == hour);
            self.coverage.observe(
                key,
                hour,
                span.map(|_| 0),
                span.is_some_and(|span| span.first_hour == hour),
            );
        }
        for (field, reads) in [
            &mut scanned.reads,
            &mut scanned.line_reads,
            &mut scanned.logistics_reads,
        ]
        .into_iter()
        .enumerate()
        {
            let key = self.key(tag, &format!("reads-{field}"));
            if reads.last() == Some(&hour) {
                self.reads.observe(key, hour, Some(i128::from(hour)), false);
                self.reads.event(key, hour, &hour)?;
            }
            reads.drain(..reads.len().saturating_sub(1));
        }
        self.values(
            tag,
            0,
            hour,
            &mut scanned.stock,
            &holder.variants,
            new_variants,
        );
        self.values(
            tag,
            1,
            hour,
            &mut scanned.daily,
            &holder.variants,
            new_variants,
        );
        self.values(
            tag,
            2,
            hour,
            &mut scanned.daily_ic,
            &holder.variants,
            new_variants,
        );
        self.values(
            tag,
            3,
            hour,
            &mut scanned.cost_low,
            &holder.variants,
            new_variants,
        );
        self.values(
            tag,
            4,
            hour,
            &mut scanned.cost_high,
            &holder.variants,
            new_variants,
        );
        self.values(
            tag,
            5,
            hour,
            &mut scanned.balance,
            &holder.logistics,
            new_logistics,
        );
        self.values(
            tag,
            6,
            hour,
            &mut scanned.logistics_stock,
            &holder.logistics,
            new_logistics,
        );
        self.values(
            tag,
            7,
            hour,
            &mut scanned.available,
            &holder.logistics,
            new_logistics,
        );
        scanned
            .ai
            .spans
            .drain(..scanned.ai.spans.len().saturating_sub(1));
        // Empty sections still cover the accepted batch, including hours where
        // every holder's table was refused or every row was absent.
        self.coverage.through(hour);
        self.reads.through(hour);
        for series in self.amounts.iter_mut().chain(self.present.iter_mut()) {
            series.through(hour);
        }
        Ok(())
    }

    fn bytes(&self) -> usize {
        self.coverage.bytes()
            + self.reads.bytes()
            + self
                .amounts
                .iter()
                .chain(self.present.iter())
                .map(Accumulator::bytes)
                .sum::<usize>()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        for (field, (amounts, present)) in self
            .amounts
            .iter_mut()
            .zip(self.present.iter_mut())
            .enumerate()
        {
            amounts.flush(
                writer,
                &format!("stockpile-{branch}-{}", FIELDS[field]),
                0,
                cancelled,
            )?;
            present.flush(
                writer,
                &format!("stockpile-{branch}-{}-present", FIELDS[field]),
                0,
                cancelled,
            )?;
        }
        self.coverage.flush(
            writer,
            &format!("stockpile-{branch}-coverage"),
            0,
            cancelled,
        )?;
        self.reads
            .flush(writer, &format!("stockpile-{branch}-reads"), 0, cancelled)
    }
}

fn observe_numeric(
    values: &mut Accumulator,
    key: u64,
    hour: i32,
    value: Option<i128>,
    starts: bool,
    boundary: bool,
) {
    // The last numeric run survives an unknown interval. Observing an equal
    // value must reactivate the key after that interval (and after a gap).
    values.observe_boundary(key, hour, value, starts, boundary);
}

/// Retrieve whole supplying intervals, including runs that cross a publication
/// boundary. Presence retains unknown rows independently from numeric values.
fn amount_runs(
    present: &mut Reader<'_>,
    amounts: &mut Reader<'_>,
    key: u64,
    from: i32,
    to: i32,
) -> io::Result<Vec<AmountRun>> {
    let mut result = BTreeMap::new();
    for clipped in present.runs(key, from, to)? {
        let run = present
            .value_at(key, clipped.first_hour)?
            .ok_or_else(|| dashboard_core::invalid("Missing stockpile supplying interval"))?;
        if run.value == 0 {
            continue;
        }
        let amount = if run.value < 0 {
            None
        } else {
            Some(
                amounts
                    .value_at(key, clipped.first_hour)?
                    .ok_or_else(|| dashboard_core::invalid("Missing stockpile amount"))?
                    .value
                    .to_string(),
            )
        };
        result.insert(
            run.first_hour,
            AmountRun {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
                amount_raw: amount,
            },
        );
    }
    Ok(result.into_values().collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    branch_id: String,
    tag: String,
    from: f64,
    to: f64,
    width: Option<u32>,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    fields: Vec<String>,
    navigation: Option<series::Navigation>,
    variant: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
    within: Option<Vec<HourSpan>>,
    within_by_tag: Option<BTreeMap<String, Vec<HourSpan>>>,
    #[serde(default)]
    included: Vec<String>,
    #[serde(default)]
    groups: Vec<String>,
}

/// Screen-sized integer operand geometry. Decimal samples keep first/last sums
/// and native need arithmetic exact before the frontend's legacy float division.
#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockpileOperandSummary {
    pub field: String,
    pub id: String,
    pub key: String,
    pub values: dashboard_base::prepared_chart::ChartSummary,
    pub presence: dashboard_base::prepared_chart::ChartSummary,
    pub samples: Vec<[Option<String>; 4]>,
    pub row_samples: Vec<[Option<i8>; 4]>,
}

#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockpileGroupSummary {
    pub key: String,
    pub operands: Vec<StockpileOperandSummary>,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockpileNamesPage {
    pub first: String,
    pub total: String,
    pub rows: Vec<StockVariantName>,
    pub incoming: Option<StockVariantName>,
    pub cursor: Option<String>,
}

#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockpileDisplay {
    pub from: f64,
    pub to: f64,
    pub stock_recorded: bool,
    pub lines_recorded: bool,
    pub balance_recorded: bool,
    pub ai_recorded: bool,
    pub operands: Vec<StockpileOperandSummary>,
    pub groups: Vec<StockpileGroupSummary>,
    pub coverage: Vec<dashboard_base::prepared_chart::ChartSummary>,
    pub nonzero_logistics: Vec<String>,
    pub choices: BTreeMap<String, choices::StockpileChoicePeak>,
    pub designs: BTreeMap<String, BTreeSet<String>>,
}

fn spans(reader: &mut Reader<'_>, key: u64, from: i32, to: i32) -> io::Result<Vec<HourSpan>> {
    let mut spans = BTreeMap::new();
    for run in reader.runs(key, from, to)? {
        let run = reader
            .value_at(key, run.first_hour)?
            .ok_or_else(|| dashboard_core::invalid("Missing stockpile coverage"))?;
        spans.insert(
            run.first_hour,
            HourSpan {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
            },
        );
    }
    Ok(spans.into_values().collect())
}

/// Native alias histories occupy disjoint dates. Compose their existing summary
/// pages with the shared ordered merge; no new grid or storage shape is needed.
fn alias_display(replies: &[serde_json::Value]) -> io::Result<Option<StockpileDisplay>> {
    use dashboard_base::prepared_chart::{ChartSummary, merge};
    fn joined(first: &mut ChartSummary, next: &ChartSummary) {
        if first.buckets.len() < next.buckets.len() {
            first.buckets.resize(next.buckets.len(), None);
        }
        for (bucket, next) in first.buckets.iter_mut().zip(&next.buckets) {
            *bucket = match (*bucket, *next) {
                (Some(a), Some(b)) if b.first_hour < a.first_hour => merge(Some(b), Some(a)),
                (Some(a), Some(b)) => merge(Some(a), Some(b)),
                (a, b) => a.or(b),
            };
        }
        match (&mut first.runs, &next.runs) {
            (Some(runs), Some(next)) => {
                runs.extend(next.iter().cloned());
                runs.sort_by_key(|run| run.first_hour);
            }
            _ => first.runs = None,
        }
    }
    fn operand(first: &mut StockpileOperandSummary, next: &StockpileOperandSummary) {
        let mut first_samples = first.values.clone();
        for (value, presence) in first_samples
            .buckets
            .iter_mut()
            .zip(&first.presence.buckets)
        {
            if value.is_none() {
                *value = *presence;
            }
        }
        let mut next_samples = next.values.clone();
        for (value, presence) in next_samples.buckets.iter_mut().zip(&next.presence.buckets) {
            if value.is_none() {
                *value = *presence;
            }
        }
        let originals = [
            (&first_samples, &first.samples, &first.row_samples),
            (&next_samples, &next.samples, &next.row_samples),
        ];
        let mut values = first.values.clone();
        joined(&mut values, &next.values);
        let mut template = first_samples.clone();
        joined(&mut template, &next_samples);
        let mut samples = Vec::new();
        let mut rows = Vec::new();
        for (index, bucket) in template.buckets.iter().enumerate() {
            let mut sample: [Option<String>; 4] = Default::default();
            let mut present: [Option<i8>; 4] = Default::default();
            if let Some(bucket) = bucket {
                for (slot, hour) in [
                    bucket.first_hour,
                    bucket.last_hour,
                    bucket.min_hour,
                    bucket.max_hour,
                ]
                .into_iter()
                .enumerate()
                {
                    for (summary, values, presence) in originals {
                        if let Some(Some(original)) = summary.buckets.get(index)
                            && let Some(at) = [
                                original.first_hour,
                                original.last_hour,
                                original.min_hour,
                                original.max_hour,
                            ]
                            .iter()
                            .position(|date| *date == hour)
                        {
                            sample[slot] = values.get(index).and_then(|row| row[at].clone());
                            present[slot] = presence.get(index).and_then(|row| row[at]);
                            break;
                        }
                    }
                }
            }
            samples.push(sample);
            rows.push(present);
        }
        first.values = values;
        first.samples = samples;
        first.row_samples = rows;
        joined(&mut first.presence, &next.presence);
    }
    let mut result: Option<StockpileDisplay> = None;
    for reply in replies {
        let next: StockpileDisplay = serde_json::from_value(reply["prepared"].clone())?;
        let Some(first) = &mut result else {
            result = Some(next);
            continue;
        };
        first.stock_recorded |= next.stock_recorded;
        first.lines_recorded |= next.lines_recorded;
        first.balance_recorded |= next.balance_recorded;
        first.ai_recorded |= next.ai_recorded;
        for (coverage, next) in first.coverage.iter_mut().zip(&next.coverage) {
            joined(coverage, next);
        }
        for next in next.operands {
            if let Some(first) = first
                .operands
                .iter_mut()
                .find(|row| row.field == next.field && row.id == next.id)
            {
                operand(first, &next);
            } else {
                first.operands.push(next);
            }
        }
        for next in next.groups {
            if let Some(first) = first.groups.iter_mut().find(|row| row.key == next.key) {
                for next in next.operands {
                    if let Some(first) = first
                        .operands
                        .iter_mut()
                        .find(|row| row.field == next.field)
                    {
                        operand(first, &next);
                    } else {
                        first.operands.push(next);
                    }
                }
            } else {
                first.groups.push(next);
            }
        }
        for (key, next) in next.choices {
            let first = first.choices.entry(key).or_default();
            let previous = first.raw.parse::<i128>().unwrap_or(0);
            let value = next
                .raw
                .parse::<i128>()
                .map_err(|_| dashboard_core::invalid("Invalid stockpile peak"))?;
            first.raw = previous.max(value).to_string();
            first.stock_drawable |= next.stock_drawable;
            first.share_drawable |= next.share_drawable;
        }
        first.nonzero_logistics.extend(next.nonzero_logistics);
        first.nonzero_logistics.sort();
        first.nonzero_logistics.dedup();
        for (id, designs) in next.designs {
            first.designs.entry(id).or_default().extend(designs);
        }
    }
    Ok(result)
}

pub fn query(
    source: &Source,
    params: &serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Option<serde_json::Value>> {
    if let Some(tags) = params.get("tags") {
        let tags: Vec<String> = serde_json::from_value(tags.clone())?;
        let mut params = params.clone();
        params
            .as_object_mut()
            .ok_or_else(|| dashboard_core::invalid("Invalid stockpile selection"))?
            .remove("tags");
        let mut replies = Vec::new();
        for tag in tags {
            dashboard_text::metadata::check_cancel(cancelled)?;
            if let Some(within) = params
                .get("withinByTag")
                .and_then(|spans| spans.get(&tag))
                .cloned()
            {
                params["within"] = within;
            } else if params.get("withinByTag").is_some() {
                params["within"] = serde_json::json!([]);
            }
            params["tag"] = serde_json::json!(tag);
            let Some(reply) = query(source, &params, context, cancelled)? else {
                return Ok(None);
            };
            replies.push(reply);
        }
        if replies
            .first()
            .is_some_and(|reply| reply.get("prepared").is_some())
            && let Some(display) = alias_display(&replies)?
        {
            replies[0]["prepared"] = serde_json::to_value(display)?;
        }
        return Ok(Some(serde_json::to_value(replies)?));
    }
    let request: Request = serde_json::from_value(params.clone())?;
    let country = CountryParams {
        branch_id: request.branch_id.clone(),
        tag: request.tag.clone(),
    };
    country.validate().map_err(dashboard_core::invalid)?;
    if !request.from.is_finite()
        || !request.to.is_finite()
        || request.from < f64::from(i32::MIN)
        || request.to > f64::from(i32::MAX)
        || request.from > request.to
        || request.width == Some(0)
    {
        return Err(dashboard_core::invalid("Invalid stockpile viewport"));
    }
    let branch = country.branch();
    let Some(coverage) = dashboard_base::prepared::coverage::coverage(
        source,
        context.content.as_ref(),
        branch,
        cancelled,
    )?
    else {
        return Ok(None);
    };
    // Final tokens and automatic choices describe the whole branch, including
    // observations after this viewport. A prefix cannot decide those facts.
    if !coverage.complete
        && (request.kind.is_empty()
            || request.kind == "window"
            || request.to.ceil() > f64::from(coverage.last_hour))
    {
        return Ok(None);
    }
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    read_window(source, request, context, &store, &key, cancelled)
}

fn read_window(
    source: &Source,
    request: Request,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Option<serde_json::Value>> {
    let from = request.from.floor() as i32;
    let to = request.to.floor() as i32;
    let country = CountryParams {
        branch_id: request.branch_id.clone(),
        tag: request.tag.clone(),
    };
    let branch = country.branch();
    let Some((keys, (holders, names, warnings, designs, first_design, choice_metadata))) =
        series::read_catalogue::<(
            BTreeMap<String, Holder>,
            VariantCatalogue,
            Vec<String>,
            BTreeMap<u64, BTreeSet<String>>,
            BTreeMap<u64, (i32, String)>,
            choices::Metadata,
        )>(store, key, "stockpile", branch)?
    else {
        return Ok(None);
    };
    let holder = holders.get(&request.tag).cloned().unwrap_or_default();
    if request.within.as_ref().is_some_and(|spans| {
        spans.iter().any(|span| span.first_hour > span.last_hour)
            || spans
                .windows(2)
                .any(|pair| pair[0].last_hour >= pair[1].first_hour)
    }) {
        return Err(dashboard_core::invalid("Invalid stockpile membership"));
    }
    let mut naming = Reader::new(store, key, format!("stockpile-{branch}-names"), cancelled)?;
    if request.kind == "names-page" {
        let variant = request
            .variant
            .as_deref()
            .ok_or_else(|| dashboard_core::invalid("Missing stockpile variant"))?
            .parse()
            .map_err(|_| dashboard_core::invalid("Invalid stockpile variant"))?;
        let cursor = request
            .cursor
            .as_deref()
            .unwrap_or("0")
            .parse()
            .map_err(|_| dashboard_core::invalid("Invalid stockpile name cursor"))?;
        let page = naming.events(&[variant], from, to, cursor, request.limit.unwrap_or(256))?;
        let mut versions = VariantCatalogue::default();
        if let Some((_, row)) = naming.latest_event(variant, from)? {
            versions
                .versions
                .entry(variant)
                .or_default()
                .push(serde_json::from_value::<VariantVersion>(row)?);
        }
        let incoming_hour = versions
            .versions
            .get(&variant)
            .and_then(|rows| rows.first())
            .map(|row| row.first_hour);
        for row in &page.rows {
            let version = serde_json::from_value::<VariantVersion>(row.row.clone())?;
            if versions
                .versions
                .get(&variant)
                .and_then(|rows| rows.last())
                .is_none_or(|last| last.first_hour != version.first_hour)
            {
                versions.versions.entry(variant).or_default().push(version);
            }
        }
        let catalogue = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options))
            .ok();
        let texts = catalogue
            .as_ref()
            .map(|catalogue| &**catalogue as &dyn Texts);
        let names = versions.names(variant, texts);
        let incoming = names
            .iter()
            .find(|name| Some(name.first_hour) == incoming_hour)
            .cloned();
        let dated: BTreeSet<_> = page.rows.iter().map(|row| row.hour).collect();
        let rows = names
            .into_iter()
            .filter(|name| dated.contains(&name.first_hour))
            .collect();
        return Ok(Some(serde_json::to_value(StockpileNamesPage {
            first: page.first.to_string(),
            total: page.total.to_string(),
            rows,
            incoming,
            cursor: page.cursor.map(|cursor| cursor.to_string()),
        })?));
    }
    if request.kind == "next-change" || request.kind == "edge" {
        let navigation = request
            .navigation
            .as_ref()
            .ok_or_else(|| dashboard_core::invalid("Missing stockpile navigation"))?;
        let mut reader = Reader::new(
            store,
            key,
            format!(
                "stockpile-{branch}-{}",
                if request.kind == "next-change" {
                    "reads"
                } else {
                    "coverage"
                }
            ),
            cancelled,
        )?;
        let fields = if request.kind == "next-change" && !request.fields.is_empty() {
            request.fields.clone()
        } else if request.kind == "next-change" {
            vec![
                "reads-0".to_owned(),
                "reads-1".to_owned(),
                "reads-2".to_owned(),
            ]
        } else {
            COVERAGE.iter().map(|field| (*field).to_owned()).collect()
        };
        let selected = fields
            .iter()
            .filter_map(|field| {
                keys.get(&series::field_key(&request.tag, field))
                    .copied()
                    .map(|key| (key, request.within.as_deref()))
            })
            .collect();
        let mut hour = series::navigate(&mut [(&mut reader, selected)], navigation)?;
        if request.kind == "edge" {
            // Edge extension walks only operand change dates. The UI evaluates
            // its existing plotted-value function and stops on a changed float.
            for (field, name) in FIELDS.iter().enumerate() {
                let selected: Vec<_> = keys
                    .iter()
                    .filter_map(|(name_key, &actor)| {
                        let (tag, stream): (String, String) =
                            serde_json::from_str(name_key).ok()?;
                        (tag == request.tag
                            && stream.starts_with(&format!("{name}:"))
                            && (request.fields.is_empty() || request.fields.contains(&stream)))
                        .then_some((actor, request.within.as_deref()))
                    })
                    .collect();
                let mut reader = Reader::new(
                    store,
                    key,
                    format!("stockpile-{branch}-{}-present", FIELDS[field]),
                    cancelled,
                )?;
                if let Some(candidate) =
                    series::navigate(&mut [(&mut reader, selected)], navigation)?
                {
                    let origin = navigation.origin()?;
                    if candidate != origin && navigation.from != Some(candidate) {
                        let forward = matches!(
                            navigation.key,
                            series::MeasureKey::Home | series::MeasureKey::ArrowRight
                        );
                        hour = Some(hour.filter(|hour| navigation.from != Some(*hour)).map_or(
                            candidate,
                            |hour| {
                                if forward {
                                    hour.min(candidate)
                                } else {
                                    hour.max(candidate)
                                }
                            },
                        ));
                    }
                }
            }
        }
        return Ok(Some(serde_json::json!({ "hour": hour })));
    }
    if !request.kind.is_empty() && request.kind != "window" {
        return Err(dashboard_core::invalid("Unknown stockpile query"));
    }
    let dense = request
        .width
        .is_some_and(|width| (i64::from(to) - i64::from(from) + 1) / i64::from(width) >= 2);
    let mut choices = choices::Query::new(
        source,
        &request,
        context,
        store,
        key,
        &keys,
        &holder,
        &choice_metadata,
        cancelled,
    )?;
    if let Some(width) = request.width {
        let numeric = (0..8)
            .map(|field| {
                let ids: Vec<_> = if field < 5 {
                    holder.variants.iter().copied().collect()
                } else {
                    holder.logistics.iter().map(|id| u64::from(*id)).collect()
                };
                ids.into_iter()
                    .filter(|id| {
                        (request.fields.is_empty()
                            || request.fields.iter().any(|name| {
                                name == FIELDS[field] || name == &format!("{}:{id}", FIELDS[field])
                            }))
                            && choices.relevant(field, *id, &request.groups, dense)
                    })
                    .count()
            })
            .sum::<usize>();
        series::summary_budget(
            request.from,
            request.to,
            width,
            numeric
                .saturating_mul(3)
                .saturating_add(request.groups.len().saturating_mul(9))
                .saturating_add(4),
        )?;
    }
    let mut scanned = ScannedStockpile {
        variants: names,
        costless: holder.costless,
        other_kinds: holder.other_kinds,
        missing_available: holder.missing_available,
        logistics_tokens: holder.tokens.clone(),
        ..Default::default()
    };
    // Names are independent pages. A window needs its incoming version and the
    // changes inside it; final catalogue tokens retain retrospective membership.
    for &id in &holder.variants {
        let mut versions = BTreeMap::new();
        if let Some(&(hour, _)) = first_design.get(&id)
            && let Some((_, row)) = naming.latest_event(id, hour)?
        {
            versions.insert(hour, serde_json::from_value::<VariantVersion>(row)?);
        }
        if let Some((hour, row)) = naming.latest_event(id, from)? {
            versions.insert(hour, serde_json::from_value::<VariantVersion>(row)?);
        }
        if let Some((hour, row)) = naming.latest_event(id, to)? {
            let version = serde_json::from_value::<VariantVersion>(row)?;
            if version.raw.is_none()
                && hour > i32::MIN
                && let Some((prior, row)) = naming.latest_event(id, hour - 1)?
            {
                versions.insert(prior, serde_json::from_value::<VariantVersion>(row)?);
            }
            versions.insert(hour, version);
        }
        if !dense {
            let mut cursor = 0;
            loop {
                let page = naming.events(&[id], from, to, cursor, 256)?;
                for row in page.rows {
                    versions.insert(row.hour, serde_json::from_value::<VariantVersion>(row.row)?);
                }
                let Some(next) = page.cursor else { break };
                cursor = next;
            }
        }
        if let Some(rows) = scanned.variants.versions.get(&id) {
            for version in rows {
                versions
                    .entry(version.first_hour)
                    .or_insert_with(|| version.clone());
            }
        }
        scanned
            .variants
            .versions
            .insert(id, versions.into_values().collect());
    }
    let mut display = StockpileDisplay {
        from: request.from,
        to: request.to,
        stock_recorded: holder.stock_read,
        lines_recorded: holder.line_read,
        balance_recorded: holder.balance_read,
        ai_recorded: holder.ai_read,
        operands: Vec::new(),
        groups: Vec::new(),
        coverage: Vec::new(),
        nonzero_logistics: Vec::new(),
        choices: BTreeMap::new(),
        designs: holder
            .variants
            .iter()
            .filter_map(|id| {
                designs
                    .get(id)
                    .cloned()
                    .map(|designs| (id.to_string(), designs))
            })
            .collect(),
    };
    let mut cover = Reader::new(
        store,
        key,
        format!("stockpile-{branch}-coverage"),
        cancelled,
    )?;
    for (field, spans_out) in [
        &mut scanned.stock.spans,
        &mut scanned.daily.spans,
        &mut scanned.balance.spans,
        &mut scanned.ai.spans,
    ]
    .into_iter()
    .enumerate()
    {
        if let Some(&actor) = keys.get(&series::field_key(&request.tag, COVERAGE[field])) {
            if let Some(width) = request.width {
                let summary = cover.summary_within(
                    actor,
                    (from, to),
                    (request.from, request.to, width),
                    0,
                    request.within.as_deref(),
                )?;
                if dense {
                    let mut known = BTreeMap::new();
                    for bucket in summary.buckets.iter().flatten() {
                        for date in [bucket.first_hour, bucket.last_hour] {
                            for span in spans(&mut cover, actor, date, date)? {
                                known.insert(span.first_hour, span);
                            }
                        }
                    }
                    *spans_out = known.into_values().collect();
                }
                display.coverage.push(summary);
            }
            if !dense {
                *spans_out = spans(&mut cover, actor, from, to)?;
            }
        }
        if let Some(within) = &request.within {
            *spans_out = spans_out
                .iter()
                .flat_map(|span| {
                    within.iter().filter_map(move |member| {
                        let first_hour = span.first_hour.max(member.first_hour);
                        let last_hour = span.last_hour.min(member.last_hour);
                        (first_hour <= last_hour).then_some(HourSpan {
                            first_hour,
                            last_hour,
                        })
                    })
                })
                .collect();
        }
    }
    let mut reads = Reader::new(store, key, format!("stockpile-{branch}-reads"), cancelled)?;
    for (field, dates) in [
        &mut scanned.reads,
        &mut scanned.line_reads,
        &mut scanned.logistics_reads,
    ]
    .into_iter()
    .enumerate()
    {
        if let Some(&actor) = keys.get(&series::field_key(&request.tag, &format!("reads-{field}")))
        {
            if let Some(row) = reads
                .events(&[actor], i32::MIN, i32::MAX, 0, 1)?
                .rows
                .first()
            {
                dates.push(row.hour);
            }
            if let Some((date, _)) = reads.latest_event(actor, from)? {
                dates.push(date);
            }
            if dense {
                if let Some((date, _)) = reads.latest_event(actor, to)?
                    && dates.last() != Some(&date)
                {
                    dates.push(date);
                }
            } else {
                let mut cursor = 0;
                loop {
                    let page = reads.events(&[actor], from, to, cursor, 256)?;
                    for row in page.rows {
                        if dates.last() != Some(&row.hour) {
                            dates.push(row.hour);
                        }
                    }
                    let Some(next) = page.cursor else { break };
                    cursor = next;
                }
            }
            dates.sort_unstable();
            dates.dedup();
        }
    }
    for (field, &field_name) in FIELDS.iter().enumerate() {
        let mut amounts = Reader::new(
            store,
            key,
            format!("stockpile-{branch}-{field_name}"),
            cancelled,
        )?;
        let mut present = Reader::new(
            store,
            key,
            format!("stockpile-{branch}-{field_name}-present"),
            cancelled,
        )?;
        let ids: Vec<u64> = if field < 5 {
            holder.variants.iter().copied().collect()
        } else {
            holder.logistics.iter().map(|id| u64::from(*id)).collect()
        };
        for id in ids {
            let requested = (request.fields.is_empty()
                || request
                    .fields
                    .iter()
                    .any(|name| name == field_name || name == &format!("{field_name}:{id}")))
                && choices.relevant(field, id, &request.groups, dense);
            let actor = keys
                .get(&series::field_key(
                    &request.tag,
                    &format!("{field_name}:{id}"),
                ))
                .copied();
            let mut runs = Vec::new();
            if let Some(actor) = actor.filter(|_| requested) {
                if let Some(width) = request.width {
                    let values = amounts.summary_within(
                        actor,
                        (from, to),
                        (request.from, request.to, width),
                        0,
                        request.within.as_deref(),
                    )?;
                    let presence = present.summary_within(
                        actor,
                        (from, to),
                        (request.from, request.to, width),
                        0,
                        request.within.as_deref(),
                    )?;
                    let mut samples = Vec::new();
                    let mut row_samples = Vec::new();
                    let mut reached = BTreeSet::new();
                    for (index, bucket) in values.buckets.iter().enumerate() {
                        let mut sample: [Option<String>; 4] = Default::default();
                        let mut rows: [Option<i8>; 4] = Default::default();
                        if let Some(bucket) = bucket.as_ref().or(presence.buckets[index].as_ref()) {
                            for (index, hour) in [
                                bucket.first_hour,
                                bucket.last_hour,
                                bucket.min_hour,
                                bucket.max_hour,
                            ]
                            .into_iter()
                            .enumerate()
                            {
                                reached.insert(hour);
                                sample[index] = amounts
                                    .value_at(actor, hour)?
                                    .map(|run| run.value.to_string());
                                rows[index] = present
                                    .value_at(actor, hour)?
                                    .map(|run| run.value.signum() as i8);
                            }
                        }
                        samples.push(sample);
                        row_samples.push(rows);
                    }
                    if dense {
                        for hour in reached {
                            runs.extend(amount_runs(
                                &mut present,
                                &mut amounts,
                                actor,
                                hour,
                                hour,
                            )?);
                        }
                        runs.sort_by_key(|run| run.first_hour);
                        runs.dedup_by_key(|run| run.first_hour);
                    }
                    display.operands.push(StockpileOperandSummary {
                        field: field_name.to_owned(),
                        id: id.to_string(),
                        key: id.to_string(),
                        values,
                        presence,
                        samples,
                        row_samples,
                    });
                }
                if !dense {
                    runs = amount_runs(&mut present, &mut amounts, actor, from, to)?;
                }
            }
            if let Some(within) = &request.within {
                runs = runs
                    .iter()
                    .flat_map(|run| {
                        within.iter().filter_map(move |member| {
                            let first_hour = run.first_hour.max(member.first_hour);
                            let last_hour = run.last_hour.min(member.last_hour);
                            (first_hour <= last_hour).then(|| AmountRun {
                                first_hour,
                                last_hour,
                                amount_raw: run.amount_raw.clone(),
                            })
                        })
                    })
                    .collect();
            }
            match field {
                0 => {
                    scanned.stock.runs.insert(id, runs);
                }
                1 => {
                    scanned.daily.runs.insert(id, runs);
                }
                2 => {
                    scanned.daily_ic.runs.insert(id, runs);
                }
                3 => {
                    scanned.cost_low.runs.insert(id, runs);
                }
                4 => {
                    scanned.cost_high.runs.insert(id, runs);
                }
                5 => {
                    scanned.balance.runs.insert(id as u32, runs);
                }
                6 => {
                    scanned.logistics_stock.runs.insert(id as u32, runs);
                }
                7 => {
                    scanned.available.runs.insert(id as u32, runs);
                }
                _ => unreachable!(),
            }
        }
    }
    scanned.stock.overflowed = holder.overflowed[0];
    scanned.daily.overflowed = holder.overflowed[1];
    scanned.daily_ic.overflowed = holder.overflowed[2];
    scanned.balance.overflowed = holder.overflowed[5];
    scanned.logistics_stock.overflowed = holder.overflowed[6];
    scanned.available.overflowed = holder.overflowed[7];
    let reply = reply(source, &country, context, &scanned, warnings, cancelled)?;
    let logistics: BTreeMap<_, _> = holder
        .logistics
        .iter()
        .zip(&reply.logistics)
        .map(|(&id, item)| (id.to_string(), item.key.clone()))
        .collect();
    display.nonzero_logistics = holder
        .nonzero_logistics
        .iter()
        .filter_map(|id| logistics.get(&id.to_string()).cloned())
        .collect();
    for operand in &mut display.operands {
        if operand.field == "balance"
            || operand.field == "logistics-stock"
            || operand.field == "available"
        {
            operand.key = logistics
                .get(&operand.id)
                .cloned()
                .ok_or_else(|| dashboard_core::invalid("Missing stockpile logistics key"))?;
        }
    }
    let default_within = [HourSpan {
        first_hour: choice_metadata.bounds().0,
        last_hour: choice_metadata.bounds().1,
    }];
    let within = request.within.as_deref().unwrap_or(&default_within);
    if request.within_by_tag.is_some() {
        let mut recorded = [false; 4];
        for (index, field) in COVERAGE.iter().enumerate() {
            if let Some(&actor) = keys.get(&series::field_key(&request.tag, field)) {
                recorded[index] = cover
                    .summary_within(
                        actor,
                        choice_metadata.bounds(),
                        (
                            f64::from(choice_metadata.bounds().0),
                            f64::from(choice_metadata.bounds().1),
                            1,
                        ),
                        0,
                        Some(within),
                    )?
                    .buckets
                    .iter()
                    .any(Option::is_some);
            }
        }
        [
            display.stock_recorded,
            display.lines_recorded,
            display.balance_recorded,
            display.ai_recorded,
        ] = recorded;
    }
    display.choices = choices.peaks(within)?;
    if let Some(width) = request.width {
        for group in &request.groups {
            if group.starts_with("[\"variant\",") {
                continue;
            }
            let mut operands = Vec::new();
            for slot in 0..3 {
                operands.push(choices.summary(
                    group,
                    slot,
                    (request.from, request.to, width),
                    within,
                )?);
            }
            display.groups.push(StockpileGroupSummary {
                key: group.clone(),
                operands,
            });
        }
    }
    let mut result = serde_json::to_value(reply)?;
    result["prepared"] = serde_json::to_value(display)?;
    Ok(Some(result))
}

#[cfg(test)]
mod tests;
