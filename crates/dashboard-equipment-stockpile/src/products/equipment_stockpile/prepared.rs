//! The shared stockpile frame step feeding integer runs and dated supplying reads.
use super::*;
use dashboard_base::prepared::frame_changes::{FrameChanges, field_mask};
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use dashboard_base::products::equipment::VariantVersion;
use replay_frame::frame::TableView;
use std::{collections::HashMap, sync::LazyLock};

/// The Countries fields a holder is read from; its id is the key.
static HOLDER_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::Countries, &["canonical_slot", "tag"]));
/// The ProductionLines fields a holder's output, IC and costs are read from;
/// the handle is the key.
static LINE_FIELDS: LazyLock<u64> = LazyLock::new(|| {
    field_mask(
        TableId::ProductionLines,
        &["country", "kind", "variant", "cost_raw", "speed_raw"],
    )
});
/// The tables a holder's numbers are read from, in the order of [`RowIndex::complete`].
const NUMERIC: [TableId; 3] = [
    TableId::Stockpiles,
    TableId::ProductionLines,
    TableId::LogisticsEquipment,
];

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
    /// Each holder's keys once allocated, so an hour finds them without
    /// spelling their names; rebuilt by use after restore.
    #[serde(skip)]
    resolved: BTreeMap<String, HolderKeys>,
    #[cfg(test)]
    #[serde(skip)]
    projected: usize,
}

/// One holder's series keys. Keys allocated together here are allocated
/// together, in this order, where they are first used, so caching them keeps
/// every key's number.
#[derive(Default)]
struct HolderKeys {
    coverage: Option<[u64; 4]>,
    reads: Option<[u64; 3]>,
    operands: [BTreeMap<u64, u64>; 8],
    complete: Option<[u64; 3]>,
    groups: BTreeMap<String, [u64; 4]>,
    known: [Option<bool>; 8],
}

/// The key named `field` of `tag`, allocated in first-use order.
fn allocate(keys: &mut BTreeMap<String, u64>, tag: &str, field: &str) -> u64 {
    let next = keys.len() as u64;
    *keys.entry(series::field_key(tag, field)).or_insert(next)
}

fn holder_keys<'a>(
    resolved: &'a mut BTreeMap<String, HolderKeys>,
    tag: &str,
) -> &'a mut HolderKeys {
    if !resolved.contains_key(tag) {
        resolved.insert(tag.to_owned(), HolderKeys::default());
    }
    resolved.get_mut(tag).expect("resolved holder keys")
}

/// What the whole tables last said of each holder's rows, kept current from
/// each hour's changed rows: the holders of each canonical slot, the raw
/// country references linked to it, each production line's country, and the
/// holders whose rows changed since their last read.
struct RowIndex {
    identities: BTreeMap<String, (u32, u32)>,
    slots: Arc<HashMap<u32, u32>>,
    holders: HashMap<u32, Vec<String>>,
    links: HashMap<u32, Vec<u32>>,
    line_owners: HashMap<u64, u32>,
    lines: HashMap<u32, BTreeSet<u64>>,
    /// Whether each [`NUMERIC`] table was complete.
    complete: [bool; 3],
    /// Slots whose stock or line rows, and whose logistics rows, changed
    /// since their holders last read them.
    stale: [BTreeSet<u32>; 2],
    views: [replay_frame::frame::SharedView; 3],
    token_owners: BTreeMap<u32, BTreeSet<u32>>,
    spellings: Option<(
        replay_frame::frame::ViewIdentity,
        BTreeMap<u32, Option<Vec<u8>>>,
    )>,
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
impl RepeatedReads {
    /// Whether a read of the tables at `reads` dates anything for the holder.
    fn repeats_any(&self, [stock_read, logistics_read]: [bool; 2]) -> bool {
        (stock_read && (self.stock || self.lines))
            || (logistics_read && self.logistics == Some(true))
    }
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
    country_rows: Option<usize>,
    #[serde(skip)]
    repeated: BTreeMap<String, RepeatedReads>,
    /// Built by each whole-table hour; none after restore.
    #[serde(skip)]
    rows: Option<RowIndex>,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
    /// Hours observed from their changed rows alone.
    #[cfg(test)]
    #[serde(skip)]
    row_hours: usize,
    /// Observe every hour from the whole tables, never from changed rows.
    #[cfg(test)]
    #[serde(skip)]
    whole_tables: bool,
}

impl Prepared {
    #[cfg(test)]
    pub fn observe(
        &mut self,
        format: replay_data::format::FormatVersion,
        frame: &DecodedFrame,
    ) -> io::Result<()> {
        self.observe_changed(
            format,
            frame,
            &FrameChanges {
                rows_changed: [true; replay_data::rows::TABLE_COUNT],
                table_read: [true; replay_data::rows::TABLE_COUNT],
                slot_read: [true; replay_data::rows::TABLE_COUNT],
                updates: None,
            },
        )
    }
    pub fn observe_changed(
        &mut self,
        format: replay_data::format::FormatVersion,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let cpu = ::replay_frame::cpu::scope_phase("stockpile:series_observe");
        self.naming.advance(frame.hour)?;
        self.numbers.advance(frame.hour)?;
        self.choices.advance(frame.hour)?;
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("stockpile:observe_apply");
        let previous = self.frontier;
        let continuous = previous.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        self.frontier = Some(frame.hour);
        let countries_changed = if changes.rows_changed[TableId::Countries.index()]
            || changes.rows_changed[TableId::CountryLinks.index()]
            || !continuous
        {
            let rows = frame.complete::<tables::Countries>().ok();
            // Recorded changes that delete no country and touch no holder
            // field keep the previous hour's holders.
            let kept = continuous
                && !changes.rows_changed[TableId::CountryLinks.index()]
                && changes
                    .rows(TableId::Countries)
                    .is_some_and(|(changed, deleted)| {
                        deleted.is_empty()
                            && changed.iter().all(|row| row.mask & *HOLDER_FIELDS == 0)
                    });
            #[cfg(test)]
            let kept = kept && !self.whole_tables;
            let same = rows.is_some_and(|rows| {
                self.country_reads.as_ref().is_some_and(|seen| {
                    (kept && self.country_rows == Some(rows.len())) || seen.matches(frame, rows)
                })
            });
            if !same {
                self.country_rows = rows.map(<[_]>::len);
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
        // Variants whose group changed this hour: their current holders regroup.
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("stockpile:presentation");
        let reclassified = if naming_changed {
            // Input changes can precede their supplying read, including after
            // restore. Retain that invalidation until the catalogue can read it;
            // the supplying-read flags still come from this accepted frame.
            let mut naming_changes = FrameChanges {
                rows_changed: changes.rows_changed,
                table_read: changes.table_read,
                slot_read: changes.slot_read,
                updates: None,
            };
            naming_changes.rows_changed[TableId::VariantDefinitions.index()] |= !self.naming_ready;
            let naming_changes = if self.naming_ready {
                changes
            } else {
                &naming_changes
            };
            let dirty: std::collections::BTreeSet<u64> = self
                .names
                .observe_changed(format, frame, naming_changes)
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
                        replay_frame::frame::TableView::Complete(_)
                    )
                });
            self.choices.classify(&self.names, &dirty, frame.hour)?;
            let reclassified: BTreeSet<u64> = dirty
                .iter()
                .copied()
                .filter(|&id| self.choices.reclassified(id, frame.hour))
                .collect();
            for id in dirty {
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
            reclassified
        } else {
            self.names.advance_changed(frame, changes);
            BTreeSet::new()
        };
        self.naming.through(frame.hour);
        drop(cpu);
        // A break, a change of holders or of the dictionary their logistics
        // tokens are spelled from is read from the whole tables.
        let whole = !continuous || countries_changed;
        #[cfg(test)]
        let whole = whole || self.whole_tables;
        if !whole && self.observe_rows(format, frame, changes, previous, &reclassified)? {
            #[cfg(test)]
            {
                self.row_hours += 1;
            }
            return Ok(());
        }
        self.observe_tables(format, frame, changes, previous)
    }

    /// Hours `first..=last` that follow the last observed, at which no field
    /// the holders or their naming read changed and no stock, logistics or
    /// variant read is dated: observing each of them only advances the
    /// series' accepted hours and the catalogue's last hour, since a holder
    /// is stepped through hours it was not visited when it is next visited.
    /// In constant time. False when the hours could only be observed from the
    /// whole tables (no row index yet), which only observation does.
    pub fn advance_quiet(&mut self, first: i32, last: i32) -> io::Result<bool> {
        #[cfg(test)]
        if self.whole_tables {
            return Ok(false);
        }
        if self.rows.is_none() || self.frontier.and_then(|hour| hour.checked_add(1)) != Some(first)
        {
            return Ok(false);
        }
        self.naming.advance_span(first, last)?;
        self.numbers.advance_span(first, last)?;
        self.choices.advance_span(first, last)?;
        self.frontier = Some(last);
        self.names.advance(last);
        self.naming.through(last);
        Ok(true)
    }

    /// Every holder from the whole tables, and the index of their rows that
    /// later hours keep current from their changed rows.
    fn observe_tables(
        &mut self,
        format: replay_data::format::FormatVersion,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        previous: Option<i32>,
    ) -> io::Result<()> {
        let cpu = ::replay_frame::cpu::scope_phase("stockpile:observe_apply");
        let reads = [
            changes.slot_read[TableId::Stockpiles.index()],
            changes.slot_read[TableId::LogisticsEquipment.index()],
        ];
        #[cfg(test)]
        let rows_before = ROW_VISITS.with(std::cell::Cell::get);
        let input = FrameRows::resolved(frame, self.country_reads.as_ref(), reads[0], reads[1]);
        for tag in input.identities.keys() {
            if !self.scan.contains_key(tag) {
                self.scan.insert(tag.clone(), ScanState::default());
            }
        }
        let touched: BTreeSet<_> = input
            .identities
            .keys()
            .cloned()
            .chain(self.active.iter().cloned())
            .collect();
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("stockpile:projection_series");
        let holders_cpu = ::replay_frame::cpu::scope_phase("stockpile:whole_holders");
        for tag in &touched {
            let identity = input.identities.get(tag).copied();
            self.observe_holder(format, &input, tag, identity, reads, previous)?;
        }
        drop(holders_cpu);
        drop(cpu);
        let _cpu = ::replay_frame::cpu::scope_phase("stockpile:pruning");
        self.repeated
            .retain(|tag, _| input.identities.contains_key(tag));
        self.active = input.identities.keys().cloned().collect();
        self.rows = RowIndex::new(frame, self.country_reads.as_ref(), input.identities, reads);
        #[cfg(test)]
        {
            self.work.row_visits += ROW_VISITS.with(std::cell::Cell::get) - rows_before;
        }
        Ok(())
    }

    /// The hour from its changed rows alone: possible when it continues the
    /// previous one with the same holders and dictionary, and each numeric
    /// table changed by exactly its recorded rows. A holder whose own rows
    /// changed since its last read of them, or whose variant changed group, is
    /// observed from its own rows; every other holder's read repeats its last
    /// one and its values carry, as they would from the whole tables. False
    /// when the whole tables are needed.
    fn observe_rows(
        &mut self,
        format: replay_data::format::FormatVersion,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        previous: Option<i32>,
        reclassified: &BTreeSet<u64>,
    ) -> io::Result<bool> {
        let cpu = ::replay_frame::cpu::scope_phase("stockpile:observe_apply");
        let reads = [
            changes.slot_read[TableId::Stockpiles.index()],
            changes.slot_read[TableId::LogisticsEquipment.index()],
        ];
        let read = reads[0] || reads[1];
        let Some(index) = self.rows.as_mut() else {
            return Ok(false);
        };
        index.mark_dictionary(frame, changes);
        if index.mark(frame, changes).is_none() {
            return Ok(false);
        }
        let mut full = BTreeMap::<String, (u32, u32)>::new();
        for (stale, _) in index.stale.iter().zip(reads).filter(|(_, due)| *due) {
            for slot in stale {
                // Dictionary owners can include slots no tag lookup selected.
                let Some(tags) = index.holders.get(slot) else {
                    continue;
                };
                for tag in tags {
                    full.insert(tag.clone(), index.identities[tag]);
                }
            }
        }
        if !reclassified.is_empty() {
            for tag in &self.active {
                let held = &self.scan[tag].result;
                if reclassified
                    .iter()
                    .any(|id| held.stock.runs.contains_key(id) || held.daily.runs.contains_key(id))
                {
                    full.insert(tag.clone(), index.identities[tag]);
                }
            }
        }
        if read {
            for (tag, repeat) in &self.repeated {
                if !full.contains_key(tag) && !repeats(repeat, index.complete, reads) {
                    full.insert(tag.clone(), index.identities[tag]);
                }
            }
        }
        #[cfg(test)]
        let rows_before = ROW_VISITS.with(std::cell::Cell::get);
        let slots: BTreeSet<u32> = full.values().map(|&(slot, _)| slot).collect();
        let Some(input) = index.gather(frame, &slots, reads) else {
            return Ok(false);
        };
        #[cfg(test)]
        {
            self.work.row_visits += ROW_VISITS.with(std::cell::Cell::get) - rows_before;
        }
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("stockpile:projection_series");
        if read {
            let _cpu = ::replay_frame::cpu::scope_phase("stockpile:repeat_reads");
            for (tag, repeat) in &self.repeated {
                if full.contains_key(tag) || !repeat.repeats_any(reads) {
                    continue;
                }
                repeat_holder(
                    self.scan.get_mut(tag).expect("active stockpile country"),
                    self.holders.get_mut(tag).expect("current stockpile holder"),
                    repeat,
                    &mut self.numbers,
                    tag,
                    frame.hour,
                    reads,
                    previous,
                )?;
            }
        }
        let holders_cpu = ::replay_frame::cpu::scope_phase("stockpile:changed_holders");
        for (tag, &identity) in &full {
            self.observe_holder(format, &input, tag, Some(identity), reads, previous)?;
        }
        drop(holders_cpu);
        drop(cpu);
        let _cpu = ::replay_frame::cpu::scope_phase("stockpile:pruning");
        let index = self.rows.as_mut().expect("stockpile row index");
        for (stale, _) in index.stale.iter_mut().zip(reads).filter(|(_, due)| *due) {
            stale.clear();
        }
        Ok(true)
    }

    /// One holder's step from its own rows of `input`, shared by the whole
    /// tables and the changed rows.
    fn observe_holder(
        &mut self,
        format: replay_data::format::FormatVersion,
        input: &FrameRows<'_>,
        tag: &str,
        identity: Option<(u32, u32)>,
        [stock_read, logistics_read]: [bool; 2],
        previous: Option<i32>,
    ) -> io::Result<()> {
        let hour = input.frame.hour;
        let _cpu = ::replay_frame::cpu::scope_phase("stockpile:holder_scan");
        let state = self.scan.get_mut(tag).expect("observed stockpile country");
        state.catch_up(previous);
        #[cfg(test)]
        {
            self.work.history_visits += 1;
        }
        let before = read_counts(&state.result);
        state.observe(format, input, identity);
        if !self.repeated.contains_key(tag) {
            self.repeated
                .insert(tag.to_owned(), RepeatedReads::default());
        }
        let repeat = self
            .repeated
            .get_mut(tag)
            .expect("repeated stockpile reads");
        if let Some((slot, _)) = identity {
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
                    .map(|rows| !rows.get(&slot).is_none_or(Vec::is_empty));
                for (index, count) in repeat.logistics_counts.iter_mut().enumerate() {
                    *count = after[index + 7] - before[index + 7];
                }
            }
        } else {
            *repeat = RepeatedReads::default();
        }
        let scanned = &mut state.result;
        if !self.holders.contains_key(tag) {
            self.holders.insert(tag.to_owned(), Holder::default());
        }
        let holder = self.holders.get_mut(tag).expect("stockpile holder");
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
        holder.variants.extend(new_variants.iter().copied());
        holder.logistics.extend(new_logistics.iter().copied());
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
            if runs
                .last()
                .is_some_and(|run| run.last_hour == hour && run.amount_raw.as_deref() != Some("0"))
            {
                holder.nonzero_logistics.insert(token);
            }
        }
        self.choices
            .observe(tag, holder, scanned, &self.names, hour, &mut self.numbers)?;
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
        self.numbers
            .observe(tag, hour, scanned, holder, &new_variants, &new_logistics)
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
        if self.choices.bind_content(content) {
            // Repeated rows can still resolve to different groups in a new binding.
            self.rows = None;
        }
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let cpu = ::replay_frame::cpu::scope_phase("stockpile:settle");
        for tag in &self.active {
            let state = self.scan.get_mut(tag).expect("active stockpile country");
            // Publication and its checkpoint hold every holder through the
            // last accepted hour, visited or not.
            state.catch_up(self.frontier);
            state.seal();
        }
        drop(cpu);
        let _cpu = ::replay_frame::cpu::scope_phase("stockpile:series_flush");
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

/// Whether a holder's read at `reads` repeats its last one: each table read
/// now was complete at that read, or is not now.
fn repeats(
    read: &RepeatedReads,
    complete: [bool; 3],
    [stock_read, logistics_read]: [bool; 2],
) -> bool {
    (!stock_read || (read.stock || !complete[0]) && (read.lines || !complete[1]))
        && (!logistics_read || read.logistics.is_some() || !complete[2])
}

/// A holder whose own rows are those of its last read: the read repeats that
/// read's dates and diagnostic counts, and every value carries. Hours that
/// read nothing for it are not visited; it is stepped through them when next
/// visited.
#[allow(clippy::too_many_arguments)]
fn repeat_holder(
    state: &mut ScanState,
    holder: &mut Holder,
    repeat: &RepeatedReads,
    numbers: &mut Numbers,
    tag: &str,
    hour: i32,
    reads: [bool; 2],
    previous: Option<i32>,
) -> io::Result<()> {
    if !repeat.repeats_any(reads) {
        return Ok(());
    }
    let [stock_read, logistics_read] = reads;
    let stock = stock_read && (repeat.stock || repeat.lines);
    let logistics = logistics_read && repeat.logistics == Some(true);
    state.catch_up(previous);
    state.advance(hour);
    let scanned = &mut state.result;
    if stock {
        if repeat.stock {
            scanned.stock.overflowed += repeat.stock_overflow;
            holder.overflowed[0] = scanned.stock.overflowed;
            scanned.reads.clear();
            scanned.reads.push(hour);
            numbers.read(tag, 0, hour)?;
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
            scanned.line_reads.push(hour);
            numbers.read(tag, 1, hour)?;
        }
    }
    if logistics {
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
        scanned.logistics_reads.push(hour);
        numbers.read(tag, 2, hour)?;
    }
    Ok(())
}

impl RowIndex {
    /// The index of the holders the whole tables just resolved; none without
    /// complete links, whose hours are all read whole. A complete table the
    /// holders did not read at `reads` may have changed since their last read
    /// of it, so every holder's next read of it is a full one.
    fn new(
        frame: &DecodedFrame,
        countries: Option<&dashboard_base::products::territory::CountryReads>,
        identities: BTreeMap<String, (u32, u32)>,
        [stock_read, logistics_read]: [bool; 2],
    ) -> Option<Self> {
        let countries = countries.filter(|_| frame.complete::<tables::CountryLinks>().is_ok())?;
        let slots = Arc::clone(countries.slots());
        let mut holders = HashMap::<u32, Vec<String>>::new();
        for (tag, &(slot, _)) in &identities {
            holders.entry(slot).or_default().push(tag.clone());
        }
        let mut links = HashMap::<u32, Vec<u32>>::new();
        for (&raw, slot) in slots.iter() {
            if holders.contains_key(slot) {
                links.entry(*slot).or_default().push(raw);
            }
        }
        for raws in links.values_mut() {
            raws.sort_unstable();
        }
        let mut line_owners = HashMap::new();
        let mut lines = HashMap::<u32, BTreeSet<u64>>::new();
        if let Ok(rows) = frame.complete::<tables::ProductionLines>() {
            for row in rows {
                #[cfg(test)]
                ROW_VISITS.with(|count| count.set(count.get() + 1));
                line_owners.insert(row.handle, row.country);
                if let Some(&slot) = slots.get(&row.country) {
                    lines.entry(slot).or_default().insert(row.handle);
                }
            }
        }
        let complete = complete(frame);
        let mut token_owners = BTreeMap::<u32, BTreeSet<u32>>::new();
        if let Ok(rows) = frame.complete::<tables::LogisticsEquipment>() {
            for row in rows {
                if let Some(&slot) = slots.get(&row.country) {
                    token_owners.entry(row.token).or_default().insert(slot);
                }
            }
        }
        let spellings = frame.complete::<tables::LexicalTokens>().ok().map(|rows| {
            (
                frame.tables[TableId::LexicalTokens.index()].identity(),
                token_owners
                    .keys()
                    .map(|&token| {
                        (
                            token,
                            rows.binary_search_by_key(&token, |row| row.token)
                                .ok()
                                .map(|at| rows[at].literal.clone()),
                        )
                    })
                    .collect(),
            )
        });
        // Only a complete table has rows that may have changed unread.
        let unread = |read: bool, complete: bool| {
            if read || !complete {
                BTreeSet::new()
            } else {
                holders.keys().copied().collect()
            }
        };
        Some(Self {
            stale: [
                unread(stock_read, complete[0] || complete[1]),
                unread(logistics_read, complete[2]),
            ],
            complete,
            identities,
            slots,
            holders,
            links,
            line_owners,
            lines,
            views: NUMERIC.map(|table| frame.tables[table.index()].clone()),
            token_owners,
            spellings,
        })
    }

    /// Spelling changes affect only the next logistics read of their owners.
    /// Keep those owners stale across hours without a supplying read.
    fn mark_dictionary(&mut self, frame: &DecodedFrame, changes: &FrameChanges) {
        let _cpu = ::replay_frame::cpu::scope_phase("stockpile:spelling_inputs");
        let view = &frame.tables[TableId::LexicalTokens.index()];
        if self
            .spellings
            .as_ref()
            .is_some_and(|(held, _)| held.is(view))
        {
            return;
        }
        let rows = frame.complete::<tables::LexicalTokens>().ok();
        if let (Some((identity, spellings)), Some(rows)) = (&mut self.spellings, rows) {
            let tokens: BTreeSet<u32> = changes.rows(TableId::LexicalTokens).map_or_else(
                || spellings.keys().copied().collect(),
                |(changed, deleted)| {
                    changed
                        .iter()
                        .map(|row| row.key as u32)
                        .chain(deleted.iter().map(|key| *key as u32))
                        .collect()
                },
            );
            for token in tokens {
                if let Some(held) = spellings.get_mut(&token) {
                    let next = rows
                        .binary_search_by_key(&token, |row| row.token)
                        .ok()
                        .map(|at| rows[at].literal.clone());
                    if *held != next {
                        self.stale[1].extend(self.token_owners[&token].iter().copied());
                        *held = next;
                    }
                }
            }
            *identity = view.identity();
        } else {
            self.stale[1].extend(self.holders.keys().copied());
            self.spellings = rows.map(|rows| {
                (
                    view.identity(),
                    self.token_owners
                        .keys()
                        .map(|&token| {
                            (
                                token,
                                rows.binary_search_by_key(&token, |row| row.token)
                                    .ok()
                                    .map(|at| rows[at].literal.clone()),
                            )
                        })
                        .collect(),
                )
            });
        }
    }

    /// Note the holders whose own rows changed this hour. None when a numeric
    /// table became or stopped being complete, changed other than by its
    /// recorded rows, or a changed line is missing from it.
    fn mark(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> Option<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("stockpile:numeric_inputs");
        if complete(frame) != self.complete {
            return None;
        }
        // Kept pre-pass frames and snapshots have no delta. Compare each
        // changed source table once; unaffected holders retain their reads.
        fn compared<T: replay_data::rows::TableSchema>(
            view: &replay_frame::frame::SharedView,
            frame: &DecodedFrame,
        ) -> Option<(Vec<replay_data::rows::DeltaRow>, Vec<u128>)> {
            let before = match &**view {
                TableView::Complete(rows) => rows.rows::<T>()?,
                _ => return None,
            };
            let now = frame.complete::<T>().ok()?;
            let keys = dashboard_base::prepared::frame_changes::differing_keys(before, now);
            let (mut changed, mut deleted) = (Vec::new(), Vec::new());
            for key in keys {
                use replay_data::rows::RowCodec;
                if now.binary_search_by_key(&key, |row| row.key()).is_ok() {
                    changed.push(replay_data::rows::DeltaRow {
                        key,
                        mask: u64::MAX,
                        format: replay_data::format::FormatVersion::CURRENT.version(),
                        recorded_fields: 0,
                        values: Vec::new(),
                    });
                } else {
                    deleted.push(key);
                }
            }
            Some((changed, deleted))
        }
        let fallback = [
            changes
                .rows(TableId::Stockpiles)
                .is_none()
                .then(|| compared::<tables::Stockpiles>(&self.views[0], frame))
                .flatten(),
            changes
                .rows(TableId::ProductionLines)
                .is_none()
                .then(|| compared::<tables::ProductionLines>(&self.views[1], frame))
                .flatten(),
            changes
                .rows(TableId::LogisticsEquipment)
                .is_none()
                .then(|| compared::<tables::LogisticsEquipment>(&self.views[2], frame))
                .flatten(),
        ];
        let [stock, lines, logistics] = std::array::from_fn(|i| {
            changes.rows(NUMERIC[i]).or_else(|| {
                fallback[i]
                    .as_ref()
                    .map(|(changed, deleted)| (changed.as_slice(), deleted.as_slice()))
            })
        });
        let (stock, lines, logistics) = (stock?, lines?, logistics?);
        let previous_lines = self.views[1].clone();
        self.views = NUMERIC.map(|table| frame.tables[table.index()].clone());
        let logistics_rows = frame.complete::<tables::LogisticsEquipment>().ok();
        for change in logistics.0 {
            if let Some(rows) = logistics_rows
                && let Ok(at) = rows.binary_search_by_key(&change.key, |row| row.key())
                && let Some(&slot) = self.slots.get(&rows[at].country)
            {
                let token = rows[at].token;
                self.token_owners.entry(token).or_default().insert(slot);
                if let Some((_, spellings)) = &mut self.spellings {
                    spellings.entry(token).or_insert_with(|| {
                        frame
                            .complete::<tables::LexicalTokens>()
                            .ok()
                            .and_then(|rows| {
                                rows.binary_search_by_key(&token, |row| row.token)
                                    .ok()
                                    .map(|at| rows[at].literal.clone())
                            })
                    });
                }
            }
        }
        // Stock and logistics keys lead with the raw country reference.
        for (table, (changed, deleted)) in [(0, stock), (1, logistics)] {
            for key in changed
                .iter()
                .map(|row| row.key)
                .chain(deleted.iter().copied())
            {
                self.stale_country(table, (key >> 32) as u32);
            }
        }
        let (changed, deleted) = lines;
        if changed.is_empty() && deleted.is_empty() {
            return Some(());
        }
        let rows = frame.complete::<tables::ProductionLines>().ok()?;
        for change in changed {
            let handle = change.key as u64;
            let known = self.line_owners.get(&handle).copied();
            if known.is_some() && change.mask & *LINE_FIELDS == 0 {
                continue;
            }
            let owner = rows[rows.binary_search_by_key(&handle, |row| row.handle).ok()?].country;
            if fallback[1].is_some()
                && let TableView::Complete(before) = &*previous_lines
                && let Some(before) = before.rows::<tables::ProductionLines>()
                && let Ok(at) = before.binary_search_by_key(&handle, |row| row.handle)
            {
                let old = &before[at];
                let now = &rows[rows.binary_search_by_key(&handle, |row| row.handle).ok()?];
                if (
                    old.country,
                    old.kind,
                    old.variant,
                    old.cost_raw,
                    old.speed_raw,
                ) == (
                    now.country,
                    now.kind,
                    now.variant,
                    now.cost_raw,
                    now.speed_raw,
                ) {
                    continue;
                }
            }
            if let Some(old) = known {
                self.stale_country(0, old);
                self.unlink(handle, old);
            }
            self.stale_country(0, owner);
            self.line_owners.insert(handle, owner);
            if let Some(&slot) = self.slots.get(&owner) {
                self.lines.entry(slot).or_default().insert(handle);
            }
        }
        for &key in deleted {
            let handle = key as u64;
            if let Some(old) = self.line_owners.remove(&handle) {
                self.stale_country(0, old);
                self.unlink(handle, old);
            }
        }
        Some(())
    }

    fn stale_country(&mut self, table: usize, raw: u32) {
        if let Some(&slot) = self.slots.get(&raw)
            && self.holders.contains_key(&slot)
        {
            self.stale[table].insert(slot);
        }
    }

    fn unlink(&mut self, handle: u64, raw: u32) {
        if let Some(slot) = self.slots.get(&raw)
            && let Some(lines) = self.lines.get_mut(slot)
        {
            lines.remove(&handle);
        }
    }

    /// The rows of `slots` in each numeric table read at `reads`, grouped as
    /// the whole tables group them: a slot's rows in key order across its
    /// links. None when an indexed line is not that slot's.
    fn gather<'a>(
        &self,
        frame: &'a DecodedFrame,
        slots: &BTreeSet<u32>,
        [stock_read, logistics_read]: [bool; 2],
    ) -> Option<FrameRows<'a>> {
        fn grouped<'a, T>(
            rows: &'a [T],
            read: bool,
            slots: &BTreeSet<u32>,
            links: &HashMap<u32, Vec<u32>>,
            country: impl Fn(&T) -> u32,
        ) -> BTreeMap<u32, Vec<&'a T>> {
            let mut result = BTreeMap::new();
            if !read {
                return result;
            }
            for slot in slots {
                let mut own = Vec::new();
                for &raw in links.get(slot).into_iter().flatten() {
                    let first = rows.partition_point(|row| country(row) < raw);
                    for row in rows[first..].iter().take_while(|row| country(row) == raw) {
                        #[cfg(test)]
                        ROW_VISITS.with(|count| count.set(count.get() + 1));
                        own.push(row);
                    }
                }
                if !own.is_empty() {
                    result.insert(*slot, own);
                }
            }
            result
        }
        let lines = match frame.complete::<tables::ProductionLines>() {
            Ok(rows) if stock_read => {
                let mut result = BTreeMap::new();
                for &slot in slots {
                    let mut own = Vec::new();
                    for handle in self.lines.get(&slot).into_iter().flatten() {
                        let row =
                            &rows[rows.binary_search_by_key(handle, |row| row.handle).ok()?];
                        if self.slots.get(&row.country) != Some(&slot) {
                            return None;
                        }
                        #[cfg(test)]
                        ROW_VISITS.with(|count| count.set(count.get() + 1));
                        own.push(row);
                    }
                    if !own.is_empty() {
                        result.insert(slot, own);
                    }
                }
                Some(result)
            }
            Ok(_) => Some(BTreeMap::new()),
            Err(_) => None,
        };
        Some(FrameRows {
            frame,
            identities: BTreeMap::new(),
            stock: frame
                .complete::<tables::Stockpiles>()
                .ok()
                .map(|rows| grouped(rows, stock_read, slots, &self.links, |row| row.country)),
            lines,
            logistics: frame
                .complete::<tables::LogisticsEquipment>()
                .ok()
                .map(|rows| grouped(rows, logistics_read, slots, &self.links, |row| row.country)),
        })
    }
}

/// Whether each [`NUMERIC`] table is complete.
fn complete(frame: &DecodedFrame) -> [bool; 3] {
    NUMERIC.map(|table| matches!(*frame.tables[table.index()], TableView::Complete(_)))
}

impl Numbers {
    fn read(&mut self, tag: &str, field: usize, hour: i32) -> io::Result<()> {
        let key = self.read_keys(tag)[field];
        self.reads.observe(key, hour, Some(i128::from(hour)), false);
        self.reads.event(key, hour, &hour)
    }
    fn read_keys(&mut self, tag: &str) -> [u64; 3] {
        let Self { keys, resolved, .. } = self;
        *holder_keys(resolved, tag).reads.get_or_insert_with(|| {
            std::array::from_fn(|field| allocate(keys, tag, &format!("reads-{field}")))
        })
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
    /// [`Self::advance`] at every hour `first..=last`, in constant time.
    fn advance_span(&mut self, first: i32, last: i32) -> io::Result<()> {
        for series in self
            .amounts
            .iter_mut()
            .chain(self.present.iter_mut())
            .chain([&mut self.coverage, &mut self.reads])
        {
            series.advance_span(first, last)?;
        }
        Ok(())
    }
    fn values<K: Ord + Copy + std::fmt::Display + Into<u64>>(
        &mut self,
        tag: &str,
        field: usize,
        hour: i32,
        rows: &mut ReadRuns<K>,
        ids: &BTreeSet<K>,
        added: &BTreeSet<K>,
    ) {
        let Self {
            keys,
            amounts,
            present,
            resolved,
            ..
        } = self;
        let changes = rows.take_changes();
        let span = rows.spans.last().filter(|span| span.last_hour == hour);
        let starts = span.is_some_and(|span| span.first_hour == hour);
        let holder = holder_keys(resolved, tag);
        let coverage_changed = holder.known[field].replace(span.is_some()) != Some(span.is_some());
        let resolved = &mut holder.operands[field];
        let changed: BTreeSet<_> = if coverage_changed || starts {
            ids.clone()
        } else if let Some(changes) = changes {
            changes.into_iter().chain(added.iter().copied()).collect()
        } else {
            // Within a continuing span a value that neither started nor ended
            // at this hour is the one already carried: only the others change.
            rows.runs
                .iter()
                .filter(|(_, runs)| {
                    runs.last()
                        .is_none_or(|run| run.last_hour != hour || run.first_hour == hour)
                })
                .map(|(&id, _)| id)
                .chain(added.iter().copied())
                .collect()
        };
        for id in &changed {
            #[cfg(test)]
            {
                self.projected += 1;
            }
            let key = *resolved
                .entry((*id).into())
                .or_insert_with(|| allocate(keys, tag, &format!("{}:{id}", FIELDS[field])));
            let Some(run) = rows
                .runs
                .get(id)
                .and_then(|runs| runs.last())
                .filter(|run| run.last_hour == hour)
            else {
                // Known absence is independently retained from an explicit
                // uncomputable row, and contributes zero to the numeric sum.
                observe_numeric(
                    &mut present[field],
                    key,
                    hour,
                    span.map(|_| 0),
                    starts,
                    false,
                );
                observe_numeric(
                    &mut amounts[field],
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
            let previous = present[field].state(key).and_then(|state| state.last);
            let version = if boundary {
                previous.map_or(1, |run| run.value.abs() + 1)
            } else {
                previous.expect("carried stockpile presence").value.abs()
            };
            observe_numeric(
                &mut present[field],
                key,
                hour,
                Some(if value.is_some() { version } else { -version }),
                starts,
                false,
            );
            observe_numeric(&mut amounts[field], key, hour, value, starts, boundary);
        }
        // ReadRuns retains its own open/carry state. Closed history is already in
        // Accumulator; retaining one current interval is sufficient for the step.
        for id in &changed {
            let Some(runs) = rows.runs.get_mut(id) else {
                continue;
            };
            if runs.last().is_none_or(|run| run.last_hour != hour) {
                rows.runs.remove(id);
            } else {
                runs.drain(..runs.len().saturating_sub(1));
            }
        }
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
        let coverage = {
            let Self { keys, resolved, .. } = self;
            *holder_keys(resolved, tag).coverage.get_or_insert_with(|| {
                std::array::from_fn(|field| allocate(keys, tag, COVERAGE[field]))
            })
        };
        for (field, spans) in [
            &scanned.stock.spans,
            &scanned.daily.spans,
            &scanned.balance.spans,
            &scanned.ai.spans,
        ]
        .into_iter()
        .enumerate()
        {
            let key = coverage[field];
            let span = spans.last().filter(|span| span.last_hour == hour);
            self.coverage.observe(
                key,
                hour,
                span.map(|_| 0),
                span.is_some_and(|span| span.first_hour == hour),
            );
        }
        let read_keys = self.read_keys(tag);
        for (field, reads) in [
            &mut scanned.reads,
            &mut scanned.line_reads,
            &mut scanned.logistics_reads,
        ]
        .into_iter()
        .enumerate()
        {
            let key = read_keys[field];
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
    // The stockpile prepared on its own answers from its own coverage.
    let coverage = dashboard_base::prepared::prefix::read_or_empty(
        source,
        context.content.as_ref(),
        "equipment-stockpile",
        branch,
        Some(request.to.ceil() as i32),
        cancelled,
    )?
    .coverage;
    let window = request.kind.is_empty() || request.kind == "window";
    // Before coverage completes, a table at one hour is the hour's own frame
    // with each holder's last reads carried to it, as preparation carries them.
    if !coverage.complete && window && request.from.floor() == request.to.floor() {
        return sample_at(source, request, context, coverage, cancelled);
    }
    // Final tokens and automatic choices describe the whole branch, including
    // observations after this viewport. A prefix cannot decide those facts.
    if !coverage.complete && (window || request.to.ceil() > f64::from(coverage.last_hour)) {
        return Ok(None);
    }
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    read_window(source, request, context, &store, &key, cancelled)
}

/// The tables the frame step reads at a sampled hour: the stockpile, line and
/// logistics tables, and the variant catalogue's with the countries and links.
fn sample_tables() -> Vec<TableId> {
    let mut tables: Vec<_> = [
        TableId::Campaign,
        TableId::Stockpiles,
        TableId::ProductionLines,
        TableId::LogisticsEquipment,
    ]
    .into_iter()
    .chain(VariantCatalogue::TABLES)
    .collect();
    tables.sort_unstable();
    tables.dedup();
    tables
}

/// The stockpile, line and logistics tables' read cadence: each is read once
/// a day, so a day before an hour holds the reads that supply it.
const READ_CADENCE_HOURS: i32 = 24;

/// The reply at one hour while the stockpile's coverage is partial, from the
/// recording (`firstView` a): the frame step over the hour's block from its
/// keyframe, and over the blocks before it while the country's stock and
/// logistics reads by the hour are not all found and the blocks stepped do
/// not yet reach a day before the hour, so each last read carries to the hour
/// as preparation carries it. The look-back is the tables' read cadence. Values at the hour are exact; variant names are the
/// hours' own, and a type is classified by its latest nonempty token by the
/// hour, which is the branch's own at its last hour. The reply carries the
/// coverage, with no hour pending; it has no prepared display, whose
/// automatic choices describe the whole branch.
fn sample_at(
    source: &Source,
    request: Request,
    context: &LoadContext,
    coverage: dashboard_base::prepared::coverage::Coverage,
    cancelled: &AtomicBool,
) -> io::Result<Option<serde_json::Value>> {
    use dashboard_base::prepared::query::observations;
    let branch: u64 = request
        .branch_id
        .parse()
        .expect("validated stockpile branch");
    let hour = request.to.floor() as i32;
    let (_, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let raw = if let Some(block) =
        source.replay_ref().index.iter().find(|block| {
            block.branch == branch && block.first_hour <= hour && hour <= block.last_hour
        }) {
        let blocks = source
            .replay_ref()
            .index
            .iter()
            .filter(|block| {
                block.branch == branch
                    && block.first_hour <= hour
                    && i64::from(block.last_hour) >= i64::from(hour) - i64::from(READ_CADENCE_HOURS)
            })
            .count();
        observations::scope(source, "stockpile:at-hour", Some(&key))
            .working_set(blocks)
            .get(
                block,
                &(&request.tag, hour, &request.within),
                cancelled,
                observations::json_bytes,
                || observe_at(source, &request, context, cancelled),
            )?
            .as_ref()
            .clone()
    } else {
        observe_at(source, &request, context, cancelled)?
    };
    let mut data = raw;
    dashboard_base::prepared::prefix::Prefix {
        coverage,
        pending: Vec::new(),
    }
    .mark(&mut data)?;
    Ok(Some(data))
}

fn observe_at(
    source: &Source,
    request: &Request,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let hour = request.to.floor() as i32;
    let country = CountryParams {
        branch_id: request.branch_id.clone(),
        tag: request.tag.clone(),
    };
    let branch = country.branch();
    let replay = source.replay_ref();
    let blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| block.branch == branch && block.first_hour <= hour)
        .collect();
    let member = request.within.as_ref().is_none_or(|within| {
        within
            .iter()
            .any(|span| span.first_hour <= hour && hour <= span.last_hour)
    });
    let selected = sample_tables();
    let step = |blocks: &[&replay::container::BlockIndex]| -> io::Result<ScannedStockpile> {
        let mut state = ScanState::default();
        for block in blocks {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let frames = replay.read_block_selected(block, &selected)?;
            let mut decoder = replay_frame::frame::FrameDecoder::for_tables(&selected);
            for frame in &frames {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let decoded = decoder.advance(frame)?;
                if decoded.hour > hour {
                    break;
                }
                let reads = decoded
                    .complete::<tables::Countries>()
                    .ok()
                    .map(|countries| {
                        dashboard_base::products::territory::CountryReads::new(decoded, countries)
                    });
                let input = FrameRows::resolved(decoded, reads.as_ref(), true, true);
                state.result.variants.observe(replay.format, decoded);
                state.observe(
                    replay.format,
                    &input,
                    input.identities.get(&request.tag).copied(),
                );
            }
        }
        Ok(state.result)
    };
    let mut scanned = ScannedStockpile::default();
    if member && let Some(mut first) = blocks.len().checked_sub(1) {
        dashboard_base::prepared::query::progressive::decodes_allowed()?;
        loop {
            scanned = step(&blocks[first..])?;
            let read = !scanned.reads.is_empty() && !scanned.logistics_reads.is_empty();
            if read
                || first == 0
                || i64::from(blocks[first].first_hour)
                    <= i64::from(hour) - i64::from(READ_CADENCE_HOURS)
            {
                break;
            }
            first -= 1;
        }
    }
    let warnings = scanned.variants.warnings();
    let (stock, daily) = (&scanned.stock.runs, &scanned.daily.runs);
    scanned
        .variants
        .versions
        .retain(|id, _| stock.contains_key(id) || daily.contains_key(id));
    let reply = reply(source, &country, context, &scanned, warnings, cancelled)?;
    let mut data = serde_json::to_value(reply)?;

    // The quantities are the supplying reads' exact operands. A later
    // nonempty variant token can still change their type attribution.
    data["approximate"] = serde_json::json!([{"firstHour": hour, "lastHour": hour}]);
    Ok(data)
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
