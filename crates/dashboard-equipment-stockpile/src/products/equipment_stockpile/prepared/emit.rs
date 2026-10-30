//! The stockpile's ordered emit of a columnar walk's hours ([`super::super::columnar`]): one
//! branch's [`Prepared`], published as production publishes it, with each holder's values kept
//! as their open runs. A holder is stepped only at an hour that can change what it publishes:
//! its identity changes, a table it reads becomes or stops being usable, its rows are read, or
//! a variant it holds changes group. Every other holder's read repeats its last one, and a
//! quiet span only advances the series.
use super::super::columnar::{self, Read, Totals};
use super::*;
use std::collections::hash_map::Entry;

/// An open run of one of a holder's values: its total (none: unknown) and its first hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Value {
    pub(super) amount: Option<i64>,
    pub(super) first: i32,
}

/// A run set's last span: its first hour, and whether it is carried.
#[derive(Clone, Copy, Serialize, Deserialize)]
struct Span {
    first: i32,
    carried: bool,
}

/// The spans of a holder's stock, lines, logistics record and AI control.
const STOCK: usize = 0;
const LINES: usize = 1;
const LOGISTICS: usize = 2;
const AI: usize = 3;

/// What a holder's last read of each table was, which a read of unchanged rows repeats.
#[derive(Default, Serialize, Deserialize)]
struct Repeat {
    stock: bool,
    lines: bool,
    /// Whether the logistics record held its rows; none where it was not complete.
    logistics: Option<bool>,
    /// The read's counts, in `read_counts`' order.
    counts: [usize; 11],
}

/// A holder's series keys, allocated where first used.
#[derive(Default, Serialize, Deserialize)]
struct Keys {
    coverage: Option<[u64; 4]>,
    reads: Option<[u64; 3]>,
    operands: [HashMap<u64, u64>; 8],
}

/// One tag's holder.
#[derive(Serialize, Deserialize)]
struct Ledger {
    tag: String,
    /// The identity this hour's holders give it.
    current: Option<(u32, u32)>,
    /// The identity it was last stepped with, and the last hour it was stepped through.
    identity: Option<(u32, u32)>,
    stepped: Option<i32>,
    spans: [Option<Span>; 4],
    stock: Vec<(u64, [Value; 1])>,
    lines: Vec<(u64, [Value; 4])>,
    logistics: Vec<(u32, [Value; 3])>,
    repeat: Repeat,
    /// Every read's counts summed, in `read_counts`' order.
    counts: [usize; 11],
    /// Its last stock, line and logistics reads.
    read: [Option<i32>; 3],
    /// Whether each field's values were known at its last step.
    known: [Option<bool>; 8],
    keys: Keys,
    grouped: choices::Grouped,
}

impl Ledger {
    fn new(tag: String) -> Self {
        Self {
            tag,
            current: None,
            identity: None,
            stepped: None,
            spans: [None; 4],
            stock: Vec::new(),
            lines: Vec::new(),
            logistics: Vec::new(),
            repeat: Repeat::default(),
            counts: [0; 11],
            read: [None; 3],
            known: [None; 8],
            keys: Keys::default(),
            grouped: choices::Grouped::default(),
        }
    }

    /// Whether it holds a stock or line run of `variant`.
    fn holds(&self, variant: u64) -> bool {
        self.stock
            .binary_search_by_key(&variant, |&(id, _)| id)
            .is_ok()
            || self
                .lines
                .binary_search_by_key(&variant, |&(id, _)| id)
                .is_ok()
    }

    /// The open run of `field`'s value of `id`.
    fn value(&self, field: usize, id: u64) -> Option<Value> {
        fn find<K: Ord + Copy, const N: usize>(
            rows: &[(K, [Value; N])],
            key: K,
            at: usize,
        ) -> Option<Value> {
            rows.binary_search_by_key(&key, |&(id, _)| id)
                .ok()
                .map(|index| rows[index].1[at])
        }
        match field {
            0 => find(&self.stock, id, 0),
            1..=4 => find(&self.lines, id, field - 1),
            _ => find(&self.logistics, u32::try_from(id).ok()?, field - 5),
        }
    }

    /// The hour the holder's read repeats its last one.
    fn repeat(
        &mut self,
        numbers: &mut Numbers,
        holder: &mut Holder,
        hour: i32,
        [stock_read, logistics_read]: [bool; 2],
    ) -> io::Result<()> {
        let stock = stock_read && (self.repeat.stock || self.repeat.lines);
        let logistics = logistics_read && self.repeat.logistics == Some(true);
        if !stock && !logistics {
            return Ok(());
        }
        self.stepped = Some(hour);
        let keys = self
            .keys
            .reads
            .ok_or_else(|| dashboard_core::invalid("A repeated stockpile read has no keys"))?;
        let mut reread = |field: usize| -> io::Result<()> {
            numbers
                .reads
                .observe(keys[field], hour, Some(i128::from(hour)), false);
            numbers.reads.event(keys[field], hour, &hour)
        };
        let mut parts = Vec::with_capacity(3);
        if stock && self.repeat.stock {
            parts.push((0, 0..1));
        }
        if stock && self.repeat.lines {
            parts.push((1, 1..7));
        }
        if logistics {
            parts.push((2, 7..11));
        }
        for (field, counts) in parts {
            for at in counts {
                self.counts[at] += self.repeat.counts[at];
            }
            self.read[field] = Some(hour);
            reread(field)?;
        }
        self.publish_counts(holder);
        Ok(())
    }

    fn publish_counts(&self, holder: &mut Holder) {
        let counts = &self.counts;
        holder.costless = counts[1];
        holder.other_kinds = counts[2];
        holder.missing_available = counts[7];
        holder.overflowed = [
            counts[0], counts[3], counts[4], counts[5], counts[6], counts[8], counts[9], counts[10],
        ];
    }
}

/// A run set's span read at `hour`: it goes on where it was carried, else starts.
fn span_read(span: &mut Option<Span>, hour: i32) {
    if !span.is_some_and(|span| span.carried) {
        *span = Some(Span {
            first: hour,
            carried: true,
        });
    }
}

/// The run set broke: nothing is carried until its next read, and every open key ends.
fn close<K: Copy + Into<u64>, const N: usize>(
    span: &mut Option<Span>,
    open: &mut Vec<(K, [Value; N])>,
    changes: &mut [Vec<u64>],
) {
    if let Some(span) = span {
        span.carried = false;
    }
    for &(key, _) in open.iter() {
        for change in changes.iter_mut() {
            change.push(key.into());
        }
    }
    open.clear();
}

/// The totals a read gives the holder: `totals` over its open values, or those values
/// where the read sent none (its rows are unchanged).
fn totals<K: Ord + Copy, const N: usize>(
    open: &[(K, [Value; N])],
    totals: Option<&Totals<K, N>>,
) -> io::Result<Vec<(K, [Option<i64>; N])>> {
    let held = |values: &[Value; N]| (*values).map(|value| value.amount);
    let Some(totals) = totals else {
        return Ok(open
            .iter()
            .map(|(key, values)| (*key, held(values)))
            .collect());
    };
    let changes = match totals {
        Totals::Whole(rows) => return Ok(rows.clone()),
        Totals::Changed(changes) => changes,
    };
    let mismatch = || dashboard_core::invalid("A stockpile read changed totals it did not hold");
    let mut next = Vec::with_capacity(open.len() + changes.len());
    let mut open = open.iter().peekable();
    for &(key, values) in changes {
        while let Some(&&(held_key, held_values)) = open.peek()
            && held_key < key
        {
            next.push((held_key, held(&held_values)));
            open.next();
        }
        let was = open.next_if(|&&(held_key, _)| held_key == key);
        match (was, values) {
            (Some((_, was)), Some(values)) if held(was) != values => next.push((key, values)),
            (None, Some(values)) => next.push((key, values)),
            (Some(_), None) => {}
            _ => return Err(mismatch()),
        }
    }
    next.extend(open.map(|(key, values)| (*key, held(values))));
    Ok(next)
}

/// A read of `now` into the open runs: a key goes on where the span was carried and its
/// total is unchanged; every other key read, and every one no longer read, changes.
fn runs_read<K: Ord + Copy + Into<u64>, const N: usize>(
    span: &mut Option<Span>,
    open: &mut Vec<(K, [Value; N])>,
    now: Vec<(K, [Option<i64>; N])>,
    hour: i32,
    changes: &mut [Vec<u64>],
) {
    let follows = span.is_some_and(|span| span.carried);
    span_read(span, hour);
    let before = std::mem::replace(open, Vec::with_capacity(now.len()));
    let mut old = before.iter().peekable();
    for (key, amounts) in now {
        while let Some(&&(gone, _)) = old.peek()
            && gone < key
        {
            for change in changes.iter_mut() {
                change.push(gone.into());
            }
            old.next();
        }
        let was = old
            .next_if(|&&(held, _)| held == key)
            .map(|&(_, values)| values);
        let mut run = [Value {
            amount: None,
            first: hour,
        }; N];
        for (at, amount) in amounts.into_iter().enumerate() {
            match was {
                Some(values) if follows && values[at].amount == amount => run[at] = values[at],
                _ => {
                    run[at].amount = amount;
                    changes[at].push(key.into());
                }
            }
        }
        open.push((key, run));
    }
    for &(gone, _) in old {
        for change in changes.iter_mut() {
            change.push(gone.into());
        }
    }
}

/// The keys of `changes` still open in `open`, ascending.
fn started<K: Ord + Copy + Into<u64>, const N: usize>(
    changes: &[Vec<u64>],
    open: &[(K, [Value; N])],
) -> Vec<u64> {
    let mut keys: Vec<u64> = changes
        .iter()
        .flatten()
        .copied()
        .filter(|&key| {
            open.binary_search_by(|&(held, _)| Into::<u64>::into(held).cmp(&key))
                .is_ok()
        })
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// The ordered emit of one branch: [`Prepared`]'s published state, with every holder's open
/// values compact.
#[derive(Serialize, Deserialize)]
pub struct Emit {
    prepared: Prepared,
    ledgers: Vec<Ledger>,
    tags: HashMap<String, usize>,
    /// The holders by canonical slot.
    slots: HashMap<u32, Vec<usize>>,
    usable: [bool; 3],
    frontier: Option<i32>,
    /// The first hour after the last gap: a holder last stepped before it goes on from
    /// nothing.
    since: i32,
    /// Quiet hours not yet advanced through.
    pending: Option<(i32, i32)>,
    visited: Vec<bool>,
    #[cfg(test)]
    #[serde(skip)]
    steps: usize,
}

impl Emit {
    /// The emit of a branch bound to `content`, as production binds its state.
    pub fn new(content: &choices::Content) -> Self {
        let mut prepared = Prepared::default();
        prepared.bind_content(content);
        Self {
            prepared,
            ledgers: Vec::new(),
            tags: HashMap::new(),
            slots: HashMap::new(),
            usable: [false; 3],
            frontier: None,
            since: i32::MIN,
            pending: None,
            visited: Vec::new(),
            #[cfg(test)]
            steps: 0,
        }
    }

    /// Writes the emit whole, for a preparation going on from it.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        serde_json::to_writer(out, self).map_err(io::Error::from)
    }

    /// The emit [`Self::save`] wrote, bound to `content` as [`Self::new`] binds it.
    pub fn restore(bytes: &[u8], content: &choices::Content) -> io::Result<Self> {
        let mut emit: Self = serde_json::from_slice(bytes)?;
        emit.prepared.bind_content(content);
        emit.prepared.choices.restored();
        Ok(emit)
    }

    /// Holders stepped, over every hour.
    #[cfg(test)]
    pub(super) fn steps(&self) -> usize {
        self.steps
    }

    /// Tags it holds a holder of.
    #[cfg(test)]
    pub(super) fn holders(&self) -> usize {
        self.ledgers.len()
    }

    /// Advances the series through the quiet hours held.
    fn settle(&mut self) -> io::Result<()> {
        let Some((first, last)) = self.pending.take() else {
            return Ok(());
        };
        let _cpu = ::replay_frame::cpu::scope_phase("stockpile:series_observe");
        let prepared = &mut self.prepared;
        prepared.naming.advance_span(first, last)?;
        prepared.numbers.advance_span(first, last)?;
        prepared.choices.advance_span(first, last)?;
        prepared.names.advance(last);
        prepared.naming.through(last);
        self.frontier = Some(last);
        Ok(())
    }

    /// One projected hour, as [`Prepared::observe_changed`] observes the frame it was
    /// projected from.
    pub fn observe(&mut self, projected: columnar::Hour) -> io::Result<()> {
        let columnar::Hour { hour, facts, .. } = projected;
        let Some(facts) = facts else {
            let last = self.pending.map_or(self.frontier, |(_, last)| Some(last));
            if last.and_then(|last| last.checked_add(1)) != Some(hour) {
                return Err(dashboard_core::invalid(
                    "A quiet stockpile hour does not follow the hour before",
                ));
            }
            match &mut self.pending {
                Some((_, last)) => *last = hour,
                None => self.pending = Some((hour, hour)),
            }
            return Ok(());
        };
        self.settle()?;
        let columnar::Facts {
            reads,
            usable,
            holders,
            evidence,
            variants,
            rows,
        } = *facts;
        let cpu = ::replay_frame::cpu::scope_phase("stockpile:series_observe");
        let prepared = &mut self.prepared;
        prepared.naming.advance(hour)?;
        prepared.numbers.advance(hour)?;
        prepared.choices.advance(hour)?;
        drop(cpu);
        if self.frontier.and_then(|last| last.checked_add(1)) != Some(hour) {
            self.since = hour;
        }
        self.frontier = Some(hour);
        let cpu = ::replay_frame::cpu::scope_phase("stockpile:presentation");
        let changed = prepared
            .names
            .observe_hour_changed(hour, evidence, |_| variants);
        let reclassified = prepared.renamed(changed, hour)?;
        prepared.naming.through(hour);
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("stockpile:observe_apply");
        let mut visits: Vec<(usize, Option<usize>)> = Vec::new();
        if let Some(holders) = &holders {
            for (index, ledger) in self.ledgers.iter_mut().enumerate() {
                let now = holders.identities.get(&ledger.tag).copied();
                if ledger.current != now {
                    ledger.current = now;
                    visits.push((index, None));
                }
            }
            self.slots.clear();
            for (tag, &identity) in &holders.identities {
                let index = match self.tags.get(tag) {
                    Some(&index) => index,
                    None => {
                        let index = self.ledgers.len();
                        let mut ledger = Ledger::new(tag.clone());
                        ledger.current = Some(identity);
                        self.ledgers.push(ledger);
                        self.tags.insert(tag.clone(), index);
                        visits.push((index, None));
                        index
                    }
                };
                self.slots.entry(identity.0).or_default().push(index);
            }
        }
        // A table becoming or stopping being usable steps every holder, as a whole hour.
        if usable != self.usable {
            self.usable = usable;
            for (index, ledger) in self.ledgers.iter().enumerate() {
                if ledger.current.is_some() || ledger.identity.is_some() {
                    visits.push((index, None));
                }
            }
        }
        for (at, read) in rows.iter().enumerate() {
            for &index in self.slots.get(&read.slot).into_iter().flatten() {
                visits.push((index, Some(at)));
            }
        }
        // A variant changing group regroups its current holders.
        if !reclassified.is_empty() {
            for (index, ledger) in self.ledgers.iter().enumerate() {
                if ledger.current.is_some() && reclassified.iter().any(|&id| ledger.holds(id)) {
                    visits.push((index, None));
                }
            }
        }
        // In tag order, as hourly observation allocates their keys.
        let ledgers = &self.ledgers;
        visits.sort_unstable_by(|a, b| ledgers[a.0].tag.cmp(&ledgers[b.0].tag).then(a.1.cmp(&b.1)));
        visits.dedup_by(|next, kept| {
            if next.0 != kept.0 {
                return false;
            }
            kept.1 = kept.1.or(next.1);
            true
        });
        drop(cpu);
        self.visited.clear();
        self.visited.resize(self.ledgers.len(), false);
        let prepared = &mut self.prepared;
        for &(index, read) in &visits {
            self.visited[index] = true;
            #[cfg(test)]
            {
                self.steps += 1;
            }
            step(
                prepared,
                &mut self.ledgers[index],
                hour,
                self.since,
                usable,
                reads,
                read.map(|at| &rows[at]),
            )?;
        }
        if reads[0] || reads[1] {
            let _cpu = ::replay_frame::cpu::scope_phase("stockpile:repeat_reads");
            for (index, ledger) in self.ledgers.iter_mut().enumerate() {
                if self.visited[index] || ledger.current.is_none() {
                    continue;
                }
                let holder = prepared.holders.get_mut(&ledger.tag).ok_or_else(|| {
                    dashboard_core::invalid("A current stockpile holder is unknown")
                })?;
                ledger.repeat(&mut prepared.numbers, holder, hour, reads)?;
            }
        }
        Ok(())
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.settle()?;
        self.prepared.flush(writer, branch, cancelled)
    }
}

/// One holder's step at `hour`, as [`Prepared::observe_holder`] steps it: from `read` where
/// its rows were read and sent, else from its open values, which a read sent nothing of holds
/// unchanged.
fn step(
    prepared: &mut Prepared,
    ledger: &mut Ledger,
    hour: i32,
    since: i32,
    usable: [bool; 3],
    [stock_read, logistics_read]: [bool; 2],
    read: Option<&Read>,
) -> io::Result<()> {
    let cpu = ::replay_frame::cpu::scope_phase("stockpile:holder_scan");
    let identity = ledger.current;
    let continues = identity.is_some()
        && ledger.identity == identity
        && ledger.stepped.is_some_and(|at| at >= since);
    let mut changes: [Vec<u64>; 8] = Default::default();
    // Each table ends its own values where it is missing; the others go on.
    if !continues || !usable[0] {
        close(
            &mut ledger.spans[STOCK],
            &mut ledger.stock,
            &mut changes[0..1],
        );
    }
    if !continues || !usable[1] {
        close(
            &mut ledger.spans[LINES],
            &mut ledger.lines,
            &mut changes[1..5],
        );
    }
    if !continues || !usable[2] {
        close(
            &mut ledger.spans[LOGISTICS],
            &mut ledger.logistics,
            &mut changes[5..8],
        );
        if let Some(span) = &mut ledger.spans[AI] {
            span.carried = false;
        }
    }
    ledger.identity = identity;
    ledger.stepped = identity.map(|_| hour);
    let unsent = || dashboard_core::invalid("A stockpile read lacks a holder whose rows it needs");
    let mut counts = [0_usize; 11];
    let mut spellings: &[(u32, String)] = &[];
    let mut ai = None;
    if identity.is_some() {
        if usable[2] && logistics_read {
            let sent = read.and_then(|read| read.logistics.as_ref());
            let carried = ledger.spans[LOGISTICS].is_some_and(|span| span.carried);
            if sent.is_none()
                && !carried
                && !(ledger.logistics.is_empty()
                    && ledger.spans[AI].is_some_and(|span| span.carried))
            {
                return Err(unsent());
            }
            let now = totals(&ledger.logistics, sent)?;
            if now.is_empty() {
                // The game computed no balance: the country was AI-controlled.
                span_read(&mut ledger.spans[AI], hour);
                close(
                    &mut ledger.spans[LOGISTICS],
                    &mut ledger.logistics,
                    &mut changes[5..8],
                );
                ai = Some(true);
            } else {
                ledger.read[2] = Some(hour);
                match read.filter(|_| sent.is_some()) {
                    Some(read) => {
                        spellings = &read.spellings;
                        counts[7..11].copy_from_slice(&read.counts[7..11]);
                    }
                    None => counts[7..11].copy_from_slice(&ledger.repeat.counts[7..11]),
                }
                runs_read(
                    &mut ledger.spans[LOGISTICS],
                    &mut ledger.logistics,
                    now,
                    hour,
                    &mut changes[5..8],
                );
                if let Some(span) = &mut ledger.spans[AI] {
                    span.carried = false;
                }
                ai = Some(false);
            }
        }
        if stock_read && usable[0] {
            let sent = read.and_then(|read| read.stock.as_ref());
            if sent.is_none() && !ledger.spans[STOCK].is_some_and(|span| span.carried) {
                return Err(unsent());
            }
            ledger.read[0] = Some(hour);
            counts[0] = match (read, sent) {
                (Some(read), Some(_)) => read.counts[0],
                _ => ledger.repeat.counts[0],
            };
            let now = totals(&ledger.stock, sent)?;
            runs_read(
                &mut ledger.spans[STOCK],
                &mut ledger.stock,
                now,
                hour,
                &mut changes[0..1],
            );
        }
        if stock_read && usable[1] {
            let sent = read.and_then(|read| read.lines.as_ref());
            if sent.is_none() && !ledger.spans[LINES].is_some_and(|span| span.carried) {
                return Err(unsent());
            }
            ledger.read[1] = Some(hour);
            match (read, sent) {
                (Some(read), Some(_)) => counts[1..7].copy_from_slice(&read.counts[1..7]),
                _ => counts[1..7].copy_from_slice(&ledger.repeat.counts[1..7]),
            }
            let now = totals(&ledger.lines, sent)?;
            runs_read(
                &mut ledger.spans[LINES],
                &mut ledger.lines,
                now,
                hour,
                &mut changes[1..5],
            );
        }
        if stock_read {
            ledger.repeat.stock = usable[0];
            ledger.repeat.lines = usable[1];
            ledger.repeat.counts[..7].copy_from_slice(&counts[..7]);
        }
        if logistics_read {
            ledger.repeat.logistics = usable[2].then_some(ai == Some(false));
            ledger.repeat.counts[7..].copy_from_slice(&counts[7..]);
        }
        for (total, count) in ledger.counts.iter_mut().zip(counts) {
            *total += count;
        }
    } else {
        ledger.repeat = Repeat::default();
    }
    let started_variants = {
        let mut keys = started(&changes[0..1], &ledger.stock);
        keys.extend(started(&changes[1..5], &ledger.lines));
        keys.sort_unstable();
        keys.dedup();
        keys
    };
    let started_logistics = started(&changes[5..8], &ledger.logistics);
    drop(cpu);
    let Prepared {
        holders,
        choices,
        names,
        numbers,
        ..
    } = prepared;
    if !holders.contains_key(&ledger.tag) {
        holders.insert(ledger.tag.clone(), Holder::default());
    }
    let holder = holders.get_mut(&ledger.tag).expect("stockpile holder");
    let new_variants: BTreeSet<u64> = started_variants
        .iter()
        .copied()
        .filter(|id| !holder.variants.contains(id))
        .collect();
    let new_logistics: BTreeSet<u64> = started_logistics
        .iter()
        .copied()
        .filter(|&id| !holder.logistics.contains(&(id as u32)))
        .collect();
    holder.variants.extend(new_variants.iter().copied());
    holder
        .logistics
        .extend(new_logistics.iter().map(|&id| id as u32));
    holder.stock_read |= ledger.read[0].is_some();
    holder.line_read |= ledger.read[1].is_some();
    holder.balance_read |= ledger.read[2].is_some();
    holder.ai_read |= ledger.spans[AI].is_some();
    // A token keeps the spelling it was first read with.
    for (token, spelling) in spellings {
        if !holder.tokens.contains_key(token) {
            holder.tokens.insert(*token, spelling.clone());
        }
    }
    for &(token, values) in &ledger.logistics {
        if values.iter().any(|value| value.amount != Some(0)) {
            holder.nonzero_logistics.insert(token);
        }
    }
    let span = |at: usize| ledger.spans[at].map(|span| (span.first, span.carried));
    choices.observe_ledger(
        holder,
        &mut ledger.grouped,
        &choices::Held {
            tag: &ledger.tag,
            stock: &ledger.stock,
            lines: &ledger.lines,
            logistics: &ledger.logistics,
            spans: [span(STOCK), span(LINES), span(LOGISTICS)],
            started: &started_variants,
            changes: &changes,
        },
        names,
        hour,
        numbers,
    )?;
    ledger.publish_counts(holder);
    observe_numbers(
        numbers,
        ledger,
        holder,
        hour,
        &changes,
        &new_variants,
        &new_logistics,
    )
}

/// [`Numbers::observe`] of a holder's step: its coverage, its reads, and each field's values
/// that started or ended a run, or all of them where its coverage changed or starts.
fn observe_numbers(
    numbers: &mut Numbers,
    ledger: &mut Ledger,
    holder: &Holder,
    hour: i32,
    changes: &[Vec<u64>; 8],
    new_variants: &BTreeSet<u64>,
    new_logistics: &BTreeSet<u64>,
) -> io::Result<()> {
    let _cpu = ::replay_frame::cpu::scope_phase("stockpile:projection_series");
    let Numbers {
        keys,
        amounts,
        present,
        coverage,
        reads,
        ..
    } = numbers;
    let tag = ledger.tag.as_str();
    let coverage_keys = *ledger
        .keys
        .coverage
        .get_or_insert_with(|| std::array::from_fn(|field| allocate(keys, tag, COVERAGE[field])));
    for (field, at) in [STOCK, LINES, LOGISTICS, AI].into_iter().enumerate() {
        let span = ledger.spans[at].filter(|span| span.carried);
        coverage.observe(
            coverage_keys[field],
            hour,
            span.map(|_| 0),
            span.is_some_and(|span| span.first == hour),
        );
    }
    let read_keys = *ledger.keys.reads.get_or_insert_with(|| {
        std::array::from_fn(|field| allocate(keys, tag, &format!("reads-{field}")))
    });
    for (field, &key) in read_keys.iter().enumerate() {
        if ledger.read[field] == Some(hour) {
            reads.observe(key, hour, Some(i128::from(hour)), false);
            reads.event(key, hour, &hour)?;
        }
    }
    for field in 0..FIELDS.len() {
        let span = ledger.spans[match field {
            0 => STOCK,
            1..=4 => LINES,
            _ => LOGISTICS,
        }]
        .filter(|span| span.carried);
        let starts = span.is_some_and(|span| span.first == hour);
        let known = span.is_some();
        let coverage_changed = ledger.known[field].replace(known) != Some(known);
        let ids: Vec<u64> = if coverage_changed || starts {
            if field < 5 {
                holder.variants.iter().copied().collect()
            } else {
                holder.logistics.iter().map(|&id| u64::from(id)).collect()
            }
        } else {
            let added = if field < 5 {
                new_variants
            } else {
                new_logistics
            };
            let mut ids: Vec<u64> = changes[field].iter().chain(added).copied().collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        for id in ids {
            let key = match ledger.keys.operands[field].entry(id) {
                Entry::Occupied(entry) => *entry.get(),
                Entry::Vacant(entry) => {
                    *entry.insert(allocate(keys, tag, &format!("{}:{id}", FIELDS[field])))
                }
            };
            let Some(value) = ledger.value(field, id) else {
                // Known absence is apart from an uncomputable row, and sums as zero.
                let absent = span.map(|_| 0);
                observe_numeric(&mut present[field], key, hour, absent, starts, false);
                observe_numeric(&mut amounts[field], key, hour, absent, starts, false);
                continue;
            };
            // A new supplying interval stays apart even when its number is the last one's.
            let boundary = value.first == hour;
            let previous = present[field].state(key).and_then(|state| state.last);
            let version = if boundary {
                previous.map_or(1, |run| run.value.abs() + 1)
            } else {
                previous
                    .ok_or_else(|| {
                        dashboard_core::invalid("A carried stockpile value has no presence")
                    })?
                    .value
                    .abs()
            };
            observe_numeric(
                &mut present[field],
                key,
                hour,
                Some(if value.amount.is_some() {
                    version
                } else {
                    -version
                }),
                starts,
                false,
            );
            observe_numeric(
                &mut amounts[field],
                key,
                hour,
                value.amount.map(i128::from),
                starts,
                boundary,
            );
        }
    }
    coverage.through(hour);
    reads.through(hour);
    for series in amounts.iter_mut().chain(present.iter_mut()) {
        series.through(hour);
    }
    Ok(())
}
