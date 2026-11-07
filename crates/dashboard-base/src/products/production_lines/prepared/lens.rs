//! Production lines folded per segment on the walk's workers. From a segment's first census on,
//! each country's lines fold into a lens of their own ([`Lenses`]), as a history reading that
//! country would fold them from a fresh start there. The ordered emit folds every history itself
//! through that census, then binds each history reading a lensed country to its lens ([`Bound`]):
//! it resolves the census the lens began from against what the history carries, and each lens
//! version the history's own last versions decide, and takes up the lens's state where the
//! lens ends. A lens stops where what it cannot know would make a history fold otherwise: its
//! country leaves its holders or links, a tag leaves it, or an output read names a line it never
//! held. Those histories fold in the emit from there.

use super::*;

/// Pages a lens never reads: its keyed values never leave it.
struct Inert;

impl Pages for Inert {
    fn write_at(&mut self, _: &str, _: u64, _: &[u8]) -> io::Result<()> {
        Err(crate::invalid("A production lens writes no pages"))
    }
    fn write(&mut self, _: &str, _: &[u8]) -> io::Result<u64> {
        Err(crate::invalid("A production lens writes no pages"))
    }
    fn read_at(&mut self, _: &str, _: u64) -> io::Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

/// The parts' id of lens `lens` in a chunk.
pub(super) fn lens_id(lens: u32) -> u64 {
    index::LENS | u64::from(lens)
}

/// `id` of a lens's own, as the history whose ids it reads from `base` on names it.
fn own(base: u64, id: u64) -> u64 {
    index::id_floor(base) | id
}

/// A version's condition: how a history's own last version decides whether it pushed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A design read where the history may hold the same.
    Design = 0,
    /// An organisation version where the history may hold the same.
    Compare = 1,
    /// An author's version, pushed only by a history holding none of that organisation.
    Author = 2,
}

impl Kind {
    fn of(kind: u8) -> io::Result<Self> {
        match kind {
            0 => Ok(Self::Design),
            1 => Ok(Self::Compare),
            2 => Ok(Self::Author),
            _ => Err(crate::invalid("Unknown production lens condition")),
        }
    }
}

/// A lens's own record of what its versions are conditional on, and the handles its lines
/// began with since its first census. Before that census rolls over it only notes what the
/// census compares: every history reading the lens compares it alike.
#[derive(Clone, Default)]
pub(super) struct Marking {
    at_start: bool,
    designs: BTreeSet<u64>,
    organisations: BTreeSet<u64>,
    next: u32,
    marks: index::Marks,
    conds: Vec<(u32, u8, u64, u32)>,
    handles: BTreeSet<u64>,
}

impl Marking {
    fn started() -> Self {
        Self {
            at_start: true,
            ..Self::default()
        }
    }

    /// The condition `variant`'s design read listed at `index` stands under: none where every
    /// history reading the lens compares it with the same last read.
    pub(super) fn design(&mut self, variant: u64, index: usize) -> Option<u32> {
        if !self.designs.insert(variant) || self.at_start {
            return None;
        }
        let id = self.take(Kind::Design, variant, index);
        self.marks.designs.push((index as u32, id));
        Some(id)
    }

    /// Whether a history may hold a version of organisation `id` the lens does not.
    pub(super) fn unsynced(&self, id: u64) -> bool {
        !self.at_start && !self.organisations.contains(&id)
    }

    /// The condition organisation `id`'s version listed at `index`, and named at `name`, stands
    /// under; an author's version leaves it as unsettled as before.
    pub(super) fn organisation(
        &mut self,
        id: u64,
        author: bool,
        index: usize,
        name: usize,
    ) -> Option<u32> {
        let settled = self.organisations.contains(&id);
        if !author {
            self.organisations.insert(id);
        }
        if self.at_start || settled {
            return None;
        }
        let kind = if author { Kind::Author } else { Kind::Compare };
        let cond = self.take(kind, id, index);
        self.marks.organisations.push((index as u32, cond));
        self.marks.names.push((name as u32, cond));
        Some(cond)
    }

    fn take(&mut self, kind: Kind, key: u64, index: usize) -> u32 {
        let id = self.next;
        self.next += 1;
        self.conds.push((id, kind as u8, key, index as u32));
        id
    }

    /// A line of `handle` began a lifetime.
    pub(super) fn began(&mut self, handle: u64) {
        if !self.at_start {
            self.handles.insert(handle);
        }
    }
}

/// A lens's open line when it began: its handle, lifetime, appearance, run, step and output
/// read.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Opened {
    pub handle: u64,
    pub lifetime: u64,
    pub appearance: u32,
    pub run: u64,
    pub step: u64,
    pub output: Option<u64>,
}

/// A lens's lines when it began, at its first census: each open line, and each handle closed
/// then with its lifetime and appearance.
#[derive(Clone, Debug, Default)]
pub(super) struct Began {
    pub lens: u32,
    pub identity: (u32, u32),
    pub hour: i32,
    pub open: Vec<Opened>,
    pub closed: Vec<(u64, u64, u32)>,
}

/// One lens's parts for a chunk, encoded, with what its binding resolves read back: its
/// versions and anchors, its conditions `(id, kind, key, index)`, and the handles its lines
/// began with.
#[derive(Clone, Debug, Default)]
pub(super) struct Piece {
    pub lens: u32,
    pub parts: Vec<(u32, u64, Vec<u8>)>,
    pub versions: index::Versions,
    pub conds: Vec<(u32, u8, u64, u32)>,
    pub handles: Vec<u64>,
}

/// A lens's state where it was cut: what a history reading it takes up there.
#[derive(Clone, Debug)]
pub(super) struct Snapshot {
    pub lens: u32,
    pub core: CoreLedger,
    pub handles: Vec<(u64, Handle)>,
}

/// The rows of lensed countries the emit's census input holds no longer: lines and output by
/// slot, modifier countries by id, modifier lines by handle.
#[derive(Clone, Debug, Default)]
pub(super) struct Drops {
    pub slots: Vec<u32>,
    pub countries: Vec<u32>,
    pub lines: Vec<u64>,
}

/// What the walk hands its emit between two hours: where a chunk ends.
#[derive(Clone, Debug, Default)]
pub(in crate::products::production_lines) struct Flush {
    /// The lenses the segment's first census began, with what the emit drops of their rows.
    pub(super) began: Vec<Began>,
    pub(super) drops: Drops,
    pub(super) pieces: Vec<Piece>,
    pub(super) snapshots: Vec<Snapshot>,
    /// Lenses no history reads past the chunk.
    pub(super) stops: Vec<u32>,
    /// Tags whose country changes at the next hour, with the lens they read before.
    pub(super) tags: Vec<(String, u32)>,
    /// The segment ends here.
    pub(super) last: bool,
}

product_codec::product! { impl Opened {
    handle: u64,
    lifetime: u64,
    appearance: u32,
    run: u64,
    step: u64,
    output: Option<u64>,
} default {} }
product_codec::product! { impl Began {
    lens: u32,
    identity: (u32, u32),
    hour: i32,
    open: Vec<Opened>,
    closed: Vec<(u64, u64, u32)>,
} default {} }
product_codec::product! { impl Piece {
    lens: u32,
    parts: Vec<(u32, u64, Vec<u8>)>,
    versions: index::Versions,
    conds: Vec<(u32, u8, u64, u32)>,
    handles: Vec<u64>,
} default {} }
product_codec::product! { impl Snapshot {
    lens: u32,
    core: CoreLedger,
    handles: Vec<(u64, Handle)>,
} default {} }
product_codec::product! { impl Drops {
    slots: Vec<u32>,
    countries: Vec<u32>,
    lines: Vec<u64>,
} default {} }
product_codec::product! { impl Flush {
    began: Vec<Began>,
    drops: Drops,
    pieces: Vec<Piece>,
    snapshots: Vec<Snapshot>,
    stops: Vec<u32>,
    tags: Vec<(String, u32)>,
    last: bool,
} default {} }

impl Flush {
    pub(in crate::products::production_lines) fn encode(&self) -> io::Result<Vec<u8>> {
        index::pack(self)
    }

    pub(super) fn decode(bytes: &[u8]) -> io::Result<Self> {
        index::unpack(bytes)
    }
}

fn part_of(part: u32) -> io::Result<index::Part> {
    use index::Part;
    Ok(match part {
        3 => Part::Overview,
        4 => Part::Detail,
        5 => Part::Closes,
        6 => Part::Versions,
        7 => Part::Countries,
        8 => Part::Modifiers,
        10 => Part::Marks,
        _ => return Err(crate::invalid("A production lens holds a part of no lens")),
    })
}

impl Piece {
    /// Its parts as a chunk holds them.
    pub(super) fn encoded(&self) -> io::Result<Vec<index::Encoded>> {
        self.parts
            .iter()
            .map(|(part, key, bytes)| {
                Ok(index::Encoded {
                    history: lens_id(self.lens),
                    part: part_of(*part)?,
                    key: *key,
                    bytes: bytes.clone(),
                })
            })
            .collect()
    }
}

/// One country's lens: its state as a fresh history reading it would hold it.
struct Lens {
    identity: (u32, u32),
    state: CountryState,
    live: bool,
}

/// Hours that read nothing and change no availability, not yet folded: their first's input
/// without its rows, through the last.
struct Span {
    quiet: HourInput,
    countries: bool,
}

/// The countries each tag named when the identities last changed, and whether the holders
/// changed since a complete read last showed them.
#[derive(Default)]
struct Seen {
    selected: BTreeMap<String, Country>,
    holders_due: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// No census yet: every history folds in the emit.
    Before,
    Folding,
    /// The segment's first census could not begin lenses.
    Off,
}

/// A segment's lenses, as its walk folds them.
pub struct Lenses {
    pages: Inert,
    trees: Trees,
    lenses: Vec<Lens>,
    by_identity: BTreeMap<(u32, u32), u32>,
    stage: Stage,
    span: Option<Span>,
    /// The last hour output was due before the first census.
    output_due: Option<i32>,
    seen: Seen,
    /// The design revision of the hour before, which a span of quiet hours never changes.
    revision: Option<u64>,
    /// The lensed countries' slots and ids, whose rows the emit does not hold.
    slots: BTreeSet<u32>,
    ids: BTreeSet<u32>,
    /// Countries whose rows the emit holds again from this hour.
    fresh: BTreeSet<(u32, u32)>,
    /// What the emit was handed since its chunk began, beyond the lenses' own.
    sent: usize,
    /// The bytes the live lenses hold, as their last fold left them.
    held: usize,
}

impl Default for Lenses {
    fn default() -> Self {
        Self::new()
    }
}

/// `input`'s availability, as a span of quiet hours joins it.
fn signature(input: &HourInput) -> (bool, bool, bool, bool, u64) {
    (
        input.lines_complete,
        input.output_complete,
        input.organisations_complete,
        input.modifiers_complete,
        input.design_revision,
    )
}

impl Lenses {
    pub fn new() -> Self {
        Self {
            pages: Inert,
            trees: Trees::new(0),
            lenses: Vec::new(),
            by_identity: BTreeMap::new(),
            stage: Stage::Before,
            span: None,
            output_due: None,
            seen: Seen::default(),
            revision: None,
            slots: BTreeSet::new(),
            ids: BTreeSet::new(),
            fresh: BTreeSet::new(),
            sent: 0,
            held: 0,
        }
    }

    /// The slots and ids of the countries whose rows the emit does not hold.
    pub(in crate::products::production_lines) fn lensed(&self) -> (&BTreeSet<u32>, &BTreeSet<u32>) {
        (&self.slots, &self.ids)
    }

    /// The countries whose rows the emit holds again from this hour, which it is handed whole.
    pub(in crate::products::production_lines) fn take_fresh(&mut self) -> BTreeSet<(u32, u32)> {
        std::mem::take(&mut self.fresh)
    }

    /// Counts what the emit was handed beyond the lenses.
    pub(in crate::products::production_lines) fn sent(&mut self, bytes: usize) {
        self.sent = self.sent.saturating_add(bytes);
    }

    /// Before `input`'s hour is folded: where the hour changes what a lens's history would
    /// fold, the chunk ends, and those lenses are cut.
    pub(in crate::products::production_lines) fn before(&mut self, input: &HourInput) -> io::Result<Option<Flush>> {
        if self.stage != Stage::Folding {
            return Ok(None);
        }
        let mut stops = BTreeSet::new();
        let mut tags = Vec::new();
        // Holders, links and tags change only where the identities do; a lens only stops.
        self.seen.holders_due |= input.identities_changed;
        if input.rows_complete && std::mem::take(&mut self.seen.holders_due) {
            for (&identity, &lens) in &self.by_identity {
                if self.lenses[lens as usize].live && !input.holders.contains_key(&identity) {
                    stops.insert(lens);
                }
            }
        }
        if input.identities_changed
            && let Some(countries) = &input.countries
        {
            for (&identity, &lens) in &self.by_identity {
                if self.lenses[lens as usize].live && !input.linked.contains(&identity) {
                    stops.insert(lens);
                }
            }
            let before = std::mem::take(&mut self.seen.selected);
            let tagged: BTreeSet<&String> = before.keys().chain(countries.keys()).collect();
            for tag in tagged {
                let (was, now) = (before.get(tag), countries.get(tag));
                if was == now {
                    continue;
                }
                if let Some(Country::Found(slot, id)) = was
                    && let Some(&lens) = self.by_identity.get(&(*slot, *id))
                    && self.lenses[lens as usize].live
                {
                    tags.push((tag.clone(), lens));
                }
                if let Some(&Country::Found(slot, id)) = now {
                    self.unlens(slot, id);
                }
            }
            self.seen.selected = countries.clone();
        }
        // An output read naming a line the lens never held may name one its history did.
        if input.lines_read
            && input.lines_complete
            && input.countries.is_some()
            && input.output_complete
        {
            for (index, lens) in self.lenses.iter().enumerate() {
                if !lens.live || stops.contains(&(index as u32)) {
                    continue;
                }
                let state = &lens.state;
                // This census's own lines are held before its output is read; a slot's rows
                // are ascending by handle.
                let lines = input
                    .lines
                    .get(&lens.identity.0)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let listed = |handle: u64| {
                    lines
                        .binary_search_by_key(&handle, |row| row.handle)
                        .is_ok_and(|at| LineKind::of(lines[at].kind).is_some())
                };
                let known = |handle: u64| {
                    listed(handle)
                        || state.open.contains_key(&handle)
                        || state.broken.contains_key(&handle)
                        || state.keys.handles.unstored(handle).is_some()
                };
                if input
                    .output
                    .get(&lens.identity.0)
                    .into_iter()
                    .flatten()
                    .chain(&input.unlinked_output)
                    .any(|row| !known(row.line))
                {
                    stops.insert(index as u32);
                }
            }
        }
        if stops.is_empty() && tags.is_empty() {
            return Ok(None);
        }
        self.fold_span()?;
        let mut flush = self.roll()?;
        let snapped: BTreeSet<u32> = stops
            .iter()
            .copied()
            .chain(tags.iter().map(|(_, lens)| *lens))
            .collect();
        for lens in snapped {
            flush.snapshots.push(self.snapshot(lens)?);
        }
        for &lens in &stops {
            let held = &mut self.lenses[lens as usize];
            held.live = false;
            let (slot, id) = held.identity;
            self.unlens(slot, id);
        }
        flush.stops = stops.into_iter().collect();
        flush.tags = tags;
        Ok(Some(flush))
    }

    /// The emit holds the rows of country `(slot, id)` again, from this hour.
    fn unlens(&mut self, slot: u32, id: u32) {
        if self.slots.remove(&slot) | self.ids.remove(&id) {
            self.fresh.insert((slot, id));
        }
    }

    /// Folds `input`'s hour: at the segment's first census its lenses begin; past it
    /// each live lens folds the hour, a chunk ending where the lenses and what the emit was
    /// handed are due.
    pub(in crate::products::production_lines) fn after(&mut self, input: &mut HourInput) -> io::Result<Option<Flush>> {
        match self.stage {
            Stage::Off => Ok(None),
            Stage::Before => {
                self.revision = Some(input.design_revision);
                let census =
                    input.lines_read && input.lines_complete && input.countries.is_some();
                if !census {
                    if input.output_read {
                        self.output_due = Some(input.hour);
                    }
                    return Ok(None);
                }
                self.begin(input).map(Some)
            }
            Stage::Folding => {
                let revision = self.revision.replace(input.design_revision);
                let quiet = !input.lines_read
                    && !input.output_read
                    && revision == Some(input.design_revision);
                let joins = quiet
                    && self.span.as_ref().is_some_and(|span| {
                        span.quiet.last_hour.checked_add(1) == Some(input.hour)
                            && signature(&span.quiet) == signature(input)
                            && span.countries == input.countries.is_some()
                    });
                if joins {
                    if let Some(span) = &mut self.span {
                        span.quiet.last_hour = input.last_hour;
                    }
                } else {
                    self.fold_span()?;
                    if quiet {
                        self.span = Some(Span {
                            quiet: input.quiet_copy(),
                            countries: input.countries.is_some(),
                        });
                    } else {
                        self.fold(input)?;
                    }
                }
                let held = self.held;
                if crate::prepared::pages::publication_due(true, false, false, || {
                    (held.saturating_add(self.sent), 0)
                }) {
                    self.fold_span()?;
                    return self.roll().map(Some);
                }
                Ok(None)
            }
        }
    }

    /// The segment's end, or where its walk failed: every live lens's last parts and state.
    pub(in crate::products::production_lines) fn end(&mut self) -> io::Result<Flush> {
        if self.stage != Stage::Folding {
            return Ok(Flush {
                last: true,
                ..Flush::default()
            });
        }
        self.fold_span()?;
        let mut flush = self.roll()?;
        for lens in 0..self.lenses.len() as u32 {
            if self.lenses[lens as usize].live {
                flush.snapshots.push(self.snapshot(lens)?);
            }
        }
        flush.last = true;
        Ok(flush)
    }

    /// Begins a lens for each linked holder at the census `input` reads, folds the census
    /// into each from a fresh start, and rolls them over to read from the hour after.
    fn begin(&mut self, input: &HourInput) -> io::Result<Flush> {
        let output_due = self.output_due.or(input.output_read.then_some(input.hour));
        // A history's census then holds unlinked lines it held open, which a lens cannot know.
        if !input.unlinked_lines.is_empty() || output_due.is_none() {
            self.stage = Stage::Off;
            return Ok(Flush::default());
        }
        let hour = input.hour;
        let mut ctx = Ctx::new(&mut self.pages, &mut self.trees, 0);
        let mut flush = Flush::default();
        for &(slot, id) in input.holders.keys() {
            if !input.linked.contains(&(slot, id)) {
                continue;
            }
            let lens = self.lenses.len() as u32;
            let mut state = CountryState {
                last_output_due: self.output_due,
                marking: Some(Box::new(Marking::started())),
                events: record::Record::journal(),
                ..CountryState::new()
            };
            state.observe_input(input, Some(Country::Found(slot, id)), &mut ctx)?;
            let open = state
                .open
                .iter()
                .map(|(&handle, line)| {
                    let run = line
                        .run
                        .as_ref()
                        .map(|run| run.id)
                        .ok_or_else(|| crate::invalid("A production lens line holds no run"))?;
                    let step = line
                        .step
                        .map(|step| step.id)
                        .ok_or_else(|| crate::invalid("A production lens line holds no step"))?;
                    Ok(Opened {
                        handle,
                        lifetime: line.lifetime,
                        appearance: line.appearance,
                        run,
                        step,
                        output: line.output.as_ref().map(|output| output.id),
                    })
                })
                .collect::<io::Result<Vec<_>>>()?;
            let closed = state
                .keys
                .handles
                .all_unstored()
                .map(|(handle, held)| (handle, held.lifetime, held.appearance))
                .collect();
            flush.began.push(Began {
                lens,
                identity: (slot, id),
                hour,
                open,
                closed,
            });
            // What it held before rolls over: a history's own chunk holds the census.
            state.take_events(true, &mut ctx)?;
            state.events.forget_taken();
            state.counters = index::Counters::default();
            if let Some(marking) = &mut state.marking {
                marking.at_start = false;
                marking.marks = index::Marks::default();
                marking.conds.clear();
                marking.handles.clear();
            }
            self.by_identity.insert((slot, id), lens);
            self.slots.insert(slot);
            self.ids.insert(id);
            if let Some(rows) = input.lines.get(&slot) {
                flush.drops.lines.extend(
                    rows.iter()
                        .map(|row| row.handle)
                        .filter(|handle| input.modifier_lines.contains_key(handle)),
                );
            }
            self.lenses.push(Lens {
                identity: (slot, id),
                state,
                live: true,
            });
        }
        flush.drops.slots = self.slots.iter().copied().collect();
        flush.drops.countries = self.ids.iter().copied().collect();
        flush.drops.lines.sort_unstable();
        self.seen = Seen {
            selected: input.countries.clone().unwrap_or_default(),
            holders_due: false,
        };
        self.stage = Stage::Folding;
        self.sent = 0;
        self.held = 0;
        Ok(flush)
    }

    /// Folds the hour `input` holds into every live lens.
    fn fold(&mut self, input: &HourInput) -> io::Result<()> {
        let mut ctx = Ctx::new(&mut self.pages, &mut self.trees, 1);
        let countries = input.countries.is_some();
        let mut held = 0_usize;
        super::super::columnar::FLOOR.add(6, 1);
        super::super::columnar::FLOOR.add(7, self.lenses.iter().filter(|lens| lens.live).count() as u64);
        for lens in self.lenses.iter_mut().filter(|lens| lens.live) {
            let (slot, id) = lens.identity;
            let selected = countries.then_some(Country::Found(slot, id));
            // A country with no line held or listed folds only what every country does.
            let idle = input.lines_read
                && lens.state.open.is_empty()
                && lens.state.broken.is_empty()
                && !input.lines.contains_key(&slot);
            if idle {
                super::super::columnar::FLOOR.add(14, 1);
            }
            let _cpu = replay_frame::cpu::scope_phase(if idle {
                "production-lines:lens_idle"
            } else {
                "production-lines:lens_lines"
            });
            lens.state.observe_input(input, selected, &mut ctx)?;
            held = held.saturating_add(lens.state.pending_bytes());
        }
        self.held = held;
        Ok(())
    }

    /// Folds the quiet hours held as one input.
    fn fold_span(&mut self) -> io::Result<()> {
        let Some(span) = self.span.take() else {
            return Ok(());
        };
        let mut ctx = Ctx::new(&mut self.pages, &mut self.trees, 1);
        let mut held = 0_usize;
        super::super::columnar::FLOOR.add(6, 1);
        super::super::columnar::FLOOR.add(7, self.lenses.iter().filter(|lens| lens.live).count() as u64);
        for lens in self.lenses.iter_mut().filter(|lens| lens.live) {
            let (slot, id) = lens.identity;
            let selected = span.countries.then_some(Country::Found(slot, id));
            lens.state.observe_input(&span.quiet, selected, &mut ctx)?;
            held = held.saturating_add(lens.state.pending_bytes());
        }
        self.held = held;
        Ok(())
    }

    /// Ends each live lens's chunk: journals its cut, what it goes on with from the next, and
    /// its piece's conditions, of which the emit makes the piece.
    fn roll(&mut self) -> io::Result<Flush> {
        let mut ctx = Ctx::new(&mut self.pages, &mut self.trees, 1);
        for lens in self.lenses.iter_mut() {
            if !lens.live {
                continue;
            }
            lens.state.materialize();
            lens.state.take_events(true, &mut ctx)?;
            let marking = lens
                .state
                .marking
                .as_mut()
                .ok_or_else(|| crate::invalid("A production lens keeps no marks"))?;
            let marks = std::mem::take(&mut marking.marks);
            let conds = std::mem::take(&mut marking.conds);
            let handles = std::mem::take(&mut marking.handles).into_iter().collect();
            lens.state.events.piece(marks, conds, handles);
        }
        self.sent = 0;
        // Each live lens's events went with its piece.
        self.held = 0;
        Ok(Flush::default())
    }

    /// What each lens journaled since last asked, encoded, in lens order: the emit makes their
    /// chunks again from it, before the flush that ends them.
    pub(in crate::products::production_lines) fn take_ops(&mut self) -> io::Result<Option<Vec<u8>>> {
        let ops: Vec<(u32, Vec<record::Op>)> = self
            .lenses
            .iter_mut()
            .enumerate()
            .filter_map(|(index, lens)| {
                let ops = lens.state.events.take_ops();
                (!ops.is_empty()).then_some((index as u32, ops))
            })
            .collect();
        if ops.is_empty() {
            return Ok(None);
        }
        index::pack(&ops).map(Some)
    }

    fn snapshot(&self, lens: u32) -> io::Result<Snapshot> {
        let state = &self.lenses[lens as usize].state;
        Ok(Snapshot {
            lens,
            core: core_ledger(state)?,
            handles: state
                .keys
                .handles
                .all_unstored()
                .map(|(handle, held)| (handle, *held))
                .collect(),
        })
    }
}

/// Lens `lens`'s piece of the chunk `events` held, with what its binding resolves.
pub(super) fn piece(
    lens: u32,
    events: Events,
    marks: index::Marks,
    conds: Vec<(u32, u8, u64, u32)>,
    handles: Vec<u64>,
) -> io::Result<Piece> {
    let id = lens_id(lens);
    let mut parts = index::encode_history(id, &events)?;
    parts.push(index::encode_part(id, index::Part::Marks, 0, &marks)?);
    Ok(Piece {
        lens,
        parts: parts
            .into_iter()
            .map(|part| (part.part as u32, part.key, part.bytes))
            .collect(),
        versions: events.versions,
        conds,
        handles,
    })
}

/// A history reading a lens: how the lens's ids read as its own, and the history's last
/// versions as the lens's resolve them.
pub(super) struct Bound {
    pub lens: u32,
    pub identity: (u32, u32),
    /// The chunk it first read the lens in: the lens's own ids read from there.
    base: u64,
    lifetimes: BTreeMap<u64, (u64, u32)>,
    runs: BTreeMap<u64, (u64, i32, bool)>,
    steps: BTreeMap<u64, u64>,
    outputs: BTreeMap<u64, u64>,
    /// How many more appearances of each handle it holds than the lens: those its census
    /// mapped, and those its lens's lines began with since, which every later binding names.
    deltas: BTreeMap<u64, i64>,
    resolved: BTreeMap<u64, i64>,
    /// Its counts when the lens began, which the lens's add to.
    counters: index::Counters,
    designs: BTreeMap<u64, Option<(DesignRead, u64)>>,
    organisations: BTreeMap<u64, Option<(RawOrganisation, u64)>>,
    /// What its first binding holds, until written.
    start: Option<index::Start>,
}

/// The counts `base` holds and `more` adds.
fn added(base: &index::Counters, more: &index::Counters) -> index::Counters {
    let mut refusals = base.refusals.clone();
    for (reason, count) in &more.refusals {
        *refusals.entry(reason.clone()).or_default() += count;
    }
    index::Counters {
        unknown_kinds: base.unknown_kinds + more.unknown_kinds,
        unlisted_output: base.unlisted_output + more.unlisted_output,
        unread_output_reads: base.unread_output_reads + more.unread_output_reads,
        unreadable_stretches: base.unreadable_stretches + more.unreadable_stretches,
        unlinked_rows: base.unlinked_rows + more.unlinked_rows,
        refusals,
        modified: base.modified || more.modified,
    }
}

/// `count` read `delta` more.
fn moved(count: u32, delta: i64) -> io::Result<u32> {
    u32::try_from(i64::from(count) + delta)
        .map_err(|_| crate::invalid("A production lens appearance reads past its history's"))
}

impl Bound {
    /// `history`, which folded the census `began` lists as its own, reading the lens from
    /// chunk `base`.
    pub(super) fn new(
        history: &mut History,
        began: &Began,
        base: u64,
        ctx: &mut Ctx<'_>,
    ) -> io::Result<Self> {
        let unbound = || crate::invalid("A production history's census differs from its lens's");
        let state = &mut history.state;
        if state.open.len() != began.open.len() || !state.broken.is_empty() {
            return Err(unbound());
        }
        let mut bound = Self {
            lens: began.lens,
            identity: began.identity,
            base,
            lifetimes: BTreeMap::new(),
            runs: BTreeMap::new(),
            steps: BTreeMap::new(),
            outputs: BTreeMap::new(),
            deltas: BTreeMap::new(),
            resolved: BTreeMap::new(),
            counters: state.counters.clone(),
            designs: BTreeMap::new(),
            organisations: BTreeMap::new(),
            start: None,
        };
        let mut start = index::Start {
            counters: state.counters.clone(),
            ..index::Start::default()
        };
        for opened in &began.open {
            let Opened {
                handle,
                lifetime,
                appearance,
                run,
                step,
                output,
            } = *opened;
            let line = state.open.get(&handle).ok_or_else(unbound)?;
            let held = line.run.as_ref().ok_or_else(unbound)?;
            let held_step = line.step.ok_or_else(unbound)?;
            bound
                .lifetimes
                .insert(lifetime, (line.lifetime, line.appearance));
            start
                .lifetimes
                .push((lifetime, line.lifetime, line.appearance));
            bound.runs.insert(
                run,
                (held.id, held.run.first_hour, held.run.starts_segment),
            );
            start.runs.push((
                run,
                held.id,
                held.run.first_hour,
                held.run.starts_segment,
            ));
            bound.steps.insert(step, held_step.id);
            start.steps.push((step, held_step.id));
            match (output, &line.output) {
                (Some(output), Some(held)) => {
                    bound.outputs.insert(output, held.id);
                    start.outputs.push((output, held.id));
                }
                (None, None) => {}
                _ => return Err(unbound()),
            }
            bound.deltas.insert(
                handle,
                i64::from(line.appearance) - i64::from(appearance),
            );
        }
        for &(handle, lifetime, appearance) in &began.closed {
            let held = state.handle(handle, ctx)?.ok_or_else(unbound)?;
            bound
                .lifetimes
                .insert(lifetime, (held.lifetime, held.appearance));
            start
                .lifetimes
                .push((lifetime, held.lifetime, held.appearance));
            bound.deltas.insert(
                handle,
                i64::from(held.appearance) - i64::from(appearance),
            );
        }
        start.appearances = bound
            .deltas
            .iter()
            .filter(|(_, delta)| **delta != 0)
            .map(|(&handle, &delta)| (handle, delta))
            .collect();
        bound.start = Some(start);
        Ok(bound)
    }

    fn lifetime(&self, id: u64) -> u64 {
        self.lifetimes
            .get(&id)
            .map_or_else(|| own(self.base, id), |held| held.0)
    }

    fn delta(&mut self, handle: u64, state: &mut CountryState, ctx: &mut Ctx<'_>) -> io::Result<i64> {
        if let Some(delta) = self.deltas.get(&handle) {
            return Ok(*delta);
        }
        let delta = i64::from(state.handle(handle, ctx)?.map_or(0, |held| held.appearance));
        self.deltas.insert(handle, delta);
        if delta != 0 {
            self.resolved.insert(handle, delta);
        }
        Ok(delta)
    }

    fn design(
        &mut self,
        variant: u64,
        state: &mut CountryState,
        ctx: &mut Ctx<'_>,
    ) -> io::Result<Option<(DesignRead, u64)>> {
        if let Some(held) = self.designs.get(&variant) {
            return Ok(*held);
        }
        let root = state.keys.root;
        let held = state.keys.designs.get(variant, root, ctx)?.copied();
        self.designs.insert(variant, held);
        Ok(held)
    }

    fn organisation(
        &mut self,
        id: u64,
        state: &mut CountryState,
        ctx: &mut Ctx<'_>,
    ) -> io::Result<Option<(RawOrganisation, u64)>> {
        if let Some(held) = self.organisations.get(&id) {
            return Ok(held.clone());
        }
        let root = state.keys.root;
        let held = state.keys.organisations.get(id, root, ctx)?.cloned();
        self.organisations.insert(id, held.clone());
        Ok(held)
    }

    /// How `history` reads `piece`, written in chunk `chunk`: which of its conditional
    /// versions the history pushed and named, where its versions began, and its handles'
    /// appearances.
    pub(super) fn bind(
        &mut self,
        history: &mut History,
        piece: &Piece,
        chunk: u64,
        ctx: &mut Ctx<'_>,
    ) -> io::Result<index::Binding> {
        let state = &mut history.state;
        let mut binding = index::Binding {
            lens: lens_id(piece.lens),
            map: self.base,
            start: self.start.take(),
            ..index::Binding::default()
        };
        for &variant in piece.versions.design_anchors.keys() {
            let anchor = self.design(variant, state, ctx)?.map(|(_, at)| at);
            binding.design_anchors.insert(variant, anchor);
        }
        for &id in piece.versions.organisation_anchors.keys() {
            let anchor = self.organisation(id, state, ctx)?.map(|(_, at)| at);
            binding.organisation_anchors.insert(id, anchor);
        }
        let mut designs = BTreeMap::new();
        let mut organisations = BTreeMap::new();
        for &(id, kind, _, index) in &piece.conds {
            match Kind::of(kind)? {
                Kind::Design => {
                    designs.insert(index, id);
                }
                kind => {
                    organisations.insert(index, (id, kind));
                }
            }
        }
        for (index, (variant, read)) in piece.versions.designs.iter().enumerate() {
            let pushed = match designs.get(&(index as u32)) {
                Some(&cond) => {
                    let pushed = self
                        .design(*variant, state, ctx)?
                        .is_none_or(|(last, _)| !same_design(&last, read));
                    if pushed {
                        binding.included.push(cond);
                    }
                    pushed
                }
                None => true,
            };
            if pushed {
                self.designs.insert(*variant, Some((*read, chunk)));
            }
        }
        for (index, (id, raw)) in piece.versions.organisations.iter().enumerate() {
            let pushed = match organisations.get(&(index as u32)) {
                Some(&(cond, kind)) => {
                    let last = self.organisation(*id, state, ctx)?;
                    let pushed = match kind {
                        Kind::Author => last.is_none(),
                        _ => last
                            .as_ref()
                            .is_none_or(|(last, _)| !same_organisation(last, raw)),
                    };
                    if pushed {
                        binding.included.push(cond);
                        if last
                            .as_ref()
                            .is_none_or(|(last, _)| !same_naming(last, raw))
                        {
                            binding.named.push(cond);
                        }
                    }
                    pushed
                }
                None => true,
            };
            if pushed {
                self.organisations
                    .insert(*id, Some((raw.clone(), chunk)));
            }
        }
        // A line the lens began since its census is read in every later chunk it is carried
        // into, whose own binding names its handle's offset too.
        for &handle in &piece.handles {
            self.delta(handle, state, ctx)?;
        }
        binding.appearances = self
            .resolved
            .iter()
            .map(|(&handle, &delta)| (handle, delta))
            .collect();
        Ok(binding)
    }

    /// `history`'s state as the lens's `snapshot` leaves it, its own design revision then
    /// `revision`, with the chunk the lens was last read in, `chunk`, holding its overview.
    pub(super) fn apply(
        &mut self,
        history: &mut History,
        snapshot: &Snapshot,
        chunk: u64,
        revision: u64,
        ctx: &mut Ctx<'_>,
    ) -> io::Result<()> {
        let core = &snapshot.core;
        let base = self.base;
        let run_of = |id: u64, mut run: RawRun, runs: &BTreeMap<u64, (u64, i32, bool)>| {
            match runs.get(&id) {
                Some(&(held, first, starts)) => {
                    run.first_hour = first;
                    run.starts_segment = starts;
                    (held, run)
                }
                None => (own(base, id), run),
            }
        };
        let lines = |held: &[(u64, LineLedger)],
                         bound: &mut Self,
                         state: &mut CountryState,
                         ctx: &mut Ctx<'_>|
         -> io::Result<BTreeMap<u64, Line>> {
            let mut lines = BTreeMap::new();
            for (handle, line) in held {
                let (lifetime, appearance) = match bound.lifetimes.get(&line.lifetime) {
                    Some(&held) => held,
                    None => (
                        own(base, line.lifetime),
                        moved(line.appearance, bound.delta(*handle, state, ctx)?)?,
                    ),
                };
                let mut restored = restore_line(*handle, line.clone())?;
                restored.lifetime = lifetime;
                restored.appearance = appearance;
                if let Some(run) = &mut restored.run {
                    let (id, value) = run_of(run.id, run.run, &bound.runs);
                    run.id = id;
                    run.run = value;
                }
                if let Some(step) = &mut restored.step {
                    step.id = bound
                        .steps
                        .get(&step.id)
                        .copied()
                        .unwrap_or_else(|| own(base, step.id));
                }
                if let Some(output) = &mut restored.output {
                    output.id = bound
                        .outputs
                        .get(&output.id)
                        .copied()
                        .unwrap_or_else(|| own(base, output.id));
                }
                lines.insert(*handle, restored);
            }
            Ok(lines)
        };
        let open = lines(&core.open, self, &mut history.state, ctx)?;
        let broken = lines(&core.broken, self, &mut history.state, ctx)?;
        let mut handles = Vec::with_capacity(snapshot.handles.len());
        for &(handle, held) in &snapshot.handles {
            let (lifetime, appearance) = match self.lifetimes.get(&held.lifetime) {
                Some(&mapped) => mapped,
                None => (
                    own(base, held.lifetime),
                    moved(held.appearance, self.delta(handle, &mut history.state, ctx)?)?,
                ),
            };
            handles.push((
                handle,
                Handle {
                    lifetime,
                    appearance,
                },
            ));
        }
        let state = &mut history.state;
        let generation = ctx.generation;
        for (handle, held) in handles {
            state.keys.handles.put(handle, held, generation);
            state.dirty.handles.insert(handle);
        }
        for (&variant, held) in &self.designs {
            if let Some((read, at)) = held
                && *at >= base
            {
                state.keys.designs.put(variant, (*read, *at), generation);
                state.dirty.designs.insert(variant);
            }
        }
        for (&id, held) in &self.organisations {
            if let Some((raw, at)) = held
                && *at >= base
            {
                state
                    .keys
                    .organisations
                    .put(id, (raw.clone(), *at), generation);
                state.dirty.organisations.insert(id);
            }
        }
        state.events = record::Record::default();
        state.last = core.last;
        state.open = open;
        state.carried = core.carried;
        state.broken = broken;
        state.last_read_identity = core.last_read_identity;
        state.broken_identity = core.broken_identity;
        state.first_read = core.first_read;
        state.output_open = core.output_open;
        state.organisations_open = core.organisations_open;
        state.last_census = core.last_census;
        state.active_variants = core.active_variants.clone();
        state.carry = core.carry;
        state.last_output_due = core.last_output_due;
        state.last_modifier_country = core
            .last_modifier_country
            .as_deref()
            .map(index::decode_country)
            .transpose()?
            .map(Arc::new);
        state.country_at = None;
        state.last_design_revision = Some(revision);
        state.counters = added(&self.counters, &core.counters);
        state.ids = Ids {
            lifetime: own(base, core.ids.0),
            run: own(base, core.ids.1),
            step: own(base, core.ids.2),
            output: own(base, core.ids.3),
        };
        state.through = core.through;
        state.overview = Some(chunk);
        state.producing.clear();
        state.changed = true;
        state.fresh = true;
        Ok(())
    }
}
