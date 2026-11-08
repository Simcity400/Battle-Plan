//! Readers that answer each production product from the indexed parts it needs: a
//! history's overview from every chunk of its view; a window of reads from the chunks its
//! hours lie in, found by bisection over the chunk table, with where what it holds ends
//! and the versions it overlays found by bisection or by where its chunks say they lie;
//! one line's modifier reads from the chunks that hold them. None assembles a whole branch
//! scan, and none walks the publications before the one it reads.

use super::index::{self, Opened, Part};
use super::prepared::{Alias, Commit, Directory, HistoryView, Identity, NativeCountry};
use super::*;
use crate::prepared::pages::Pages;
use crate::products::country_groups::HourSpan;
use replay_data::rows::CountryProductionModifiersRow;
use std::collections::HashMap;

/// The latest commit: its range and record.
pub(super) fn commit(
    pages: &mut dyn Pages,
    branch: u64,
    commits: u64,
) -> io::Result<(u64, Commit)> {
    let range = commits
        .checked_sub(1)
        .ok_or_else(|| crate::invalid("Production lines are still being prepared"))?;
    let bytes = pages
        .read_at(&index::commits_section(branch), range)?
        .ok_or_else(|| crate::invalid("Missing production commit"))?;
    Ok((range, index::unpack(&bytes)?))
}

/// One chunk of the published sequence: its range and the hours its flush covered.
#[derive(Clone, Copy, Debug)]
pub(super) struct Chunk {
    pub range: u64,
    pub first: i32,
    pub last: i32,
}

/// What one publication names: its commit and the identities a query resolves, which the
/// query cache keeps per commit.
pub(super) struct Published {
    pub commit: Commit,
    pub identity: Identity,
    histories: BTreeMap<u64, HistoryView>,
}

impl Published {
    pub fn read(pages: &mut dyn Pages, directory: &Directory) -> io::Result<Self> {
        let branch = directory.branch;
        let (_, commit) = commit(pages, branch, directory.commits)?;
        if commit.tail.len() as u64 != commit.chunks % index::CHUNK_PAGE as u64 {
            return Err(crate::invalid(
                "A production commit's chunk table does not add up",
            ));
        }
        if !index::spans_add_up(commit.spans, commit.span_tail.len()) {
            return Err(crate::invalid(
                "A production commit's unread spans do not add up",
            ));
        }
        let bytes = pages
            .read_at(&index::ledger_section(branch), commit.identity)?
            .ok_or_else(|| crate::invalid("Missing production identities"))?;
        let identity: Identity = index::unpack(&bytes)?;
        let histories = identity
            .histories
            .iter()
            .map(|view| (view.id, view.clone()))
            .collect();
        Ok(Self {
            commit,
            identity,
            histories,
        })
    }
}

/// The chunks a view reads, `(owner, from, until)` in order, each from its owner's parts.
type Segments = Vec<(u64, u64, u64)>;

/// The history whose parts the view reads at `chunk`.
fn owner(segments: &Segments, chunk: u64) -> io::Result<u64> {
    segments
        .iter()
        .find(|(_, from, until)| *from <= chunk && chunk < *until)
        .map(|(owner, _, _)| *owner)
        .ok_or_else(|| crate::invalid("Production view reads no chunk there"))
}

/// Every chunk a view reads, in order, with its owner.
fn walk(segments: &Segments) -> impl Iterator<Item = (u64, u64)> + '_ {
    segments
        .iter()
        .flat_map(|&(owner, from, until)| (from..until).map(move |chunk| (chunk, owner)))
}

/// The icons the census inputs held over a branch: each variant's from an hour on, and the
/// hours an input began anew holding none.
#[derive(Default)]
struct Icons {
    changes: HashMap<u64, Vec<(i32, IconChoice)>>,
    resets: Vec<i32>,
}

impl Icons {
    /// `held`'s icon at `hour`: the last chosen by then, unless an input began anew after it.
    fn at<'a>(&self, held: &'a [(i32, IconChoice)], hour: i32) -> Option<&'a IconChoice> {
        let at = held.partition_point(|(changed, _)| *changed <= hour);
        let (changed, icon) = held.get(at.checked_sub(1)?)?;
        let reset = self.resets.partition_point(|reset| *reset <= hour);
        let cleared = reset
            .checked_sub(1)
            .is_some_and(|reset| self.resets[reset] > *changed);
        (!cleared).then_some(icon)
    }
}

/// The variant versions and icon events every chunk holds, folded in order. It holds no
/// country's lines, so a query keeps it per chunk identity.
pub(super) struct VariantFold {
    pub catalogue: VariantCatalogue,
    pub icons: Vec<(u64, i32, IconChoice, bool)>,
}

/// A line of `production-lines`: its life and steps, without its reads.
#[derive(Clone, Debug)]
pub(super) struct OverviewLine {
    pub lifetime: u64,
    pub key: String,
    pub handle: u64,
    pub kind: Option<LineKind>,
    pub steps: Vec<index::Step>,
    pub final_output: Option<RawOutput>,
    pub ended_by: Option<i32>,
    pub before_recording: bool,
    pub between_reads: Option<HourSpan>,
    pub variants: Vec<u64>,
}

/// A history's whole branch as `production-lines` sends it.
#[derive(Clone, Debug, Default)]
pub(super) struct Overview {
    pub lines: Vec<OverviewLine>,
    pub reads: Vec<i32>,
    pub output_hours: Vec<HourSpan>,
    pub line_hours: Vec<HourSpan>,
    pub organisation_hours: Vec<HourSpan>,
    pub unread_output: Vec<HourSpan>,
    /// Every variant a step, an output-only line or an output read names.
    pub used: BTreeSet<u64>,
    /// Each organisation's versions where its naming operands change.
    pub organisations: BTreeMap<u64, Vec<RawOrganisation>>,
    /// Each variant's icons as the census reads chose them, before the variant events'.
    pub icons: BTreeMap<u64, Vec<IconAt>>,
    pub counters: index::Counters,
}

/// A line's reads holding any hour of a window, whole.
#[derive(Clone, Debug)]
pub(super) struct WindowLine {
    pub lifetime: u64,
    pub key: String,
    pub runs: Vec<RawRun>,
    pub output: Vec<RawOutput>,
}

/// What `production-line-detail` sends of a window: the lines with reads in it, and the
/// design and organisation versions those reads overlay.
#[derive(Clone, Debug, Default)]
pub(super) struct Window {
    pub lines: Vec<WindowLine>,
    pub designs: BTreeMap<u64, Vec<DesignRead>>,
    pub organisations: BTreeMap<u64, Vec<RawOrganisation>>,
}

/// Applies the variant events' icons to a history's icon versions: each variant a census
/// gave an icon takes an event's choice where its definition changed, or where the one
/// before was waiting for that definition.
pub(super) fn merge_icons(
    icons: &mut BTreeMap<u64, Vec<IconAt>>,
    events: &[(u64, i32, IconChoice, bool)],
    cancelled: &AtomicBool,
) -> io::Result<()> {
    for (id, hour, icon, definition_changed) in events {
        crate::metadata::check_cancel(cancelled)?;
        let (id, hour, definition_changed) = (*id, *hour, *definition_changed);
        if let Some(versions) = icons.get_mut(&id)
            && versions.first().is_some_and(|first| first.0 <= hour)
        {
            let at = versions.partition_point(|(at, _)| *at <= hour);
            let pending = at != 0
                && versions[at - 1]
                    .1
                    .otherwise
                    .as_ref()
                    .err()
                    .is_some_and(|reason| {
                        matches!(
                            reason.as_str(),
                            "the variant's definition is not recorded yet"
                                | "the variant's definition is not recorded at this time"
                        )
                    });
            if (definition_changed || pending) && (at == 0 || versions[at - 1].1 != *icon) {
                versions.insert(at, (hour, icon.clone()));
            }
        }
    }
    for versions in icons.values_mut() {
        versions.dedup_by(|a, b| a.1 == b.1);
    }
    Ok(())
}

/// The window's lines in first-seen order, each with its reads in order.
fn window_lines(
    runs: BTreeMap<u64, (u64, RawRun, bool)>,
    outputs: BTreeMap<u64, (u64, RawOutput, bool)>,
    lives: &BTreeMap<u64, (u64, u32)>,
) -> io::Result<Vec<WindowLine>> {
    let mut lines: BTreeMap<u64, WindowLine> = BTreeMap::new();
    let line = |lines: &mut BTreeMap<u64, WindowLine>, lifetime: u64| -> io::Result<()> {
        if !lines.contains_key(&lifetime) {
            let (handle, appearance) = lives
                .get(&lifetime)
                .copied()
                .ok_or_else(|| crate::invalid("Production read names no lifetime"))?;
            lines.insert(
                lifetime,
                WindowLine {
                    lifetime,
                    key: format!("{handle}#{appearance}"),
                    runs: Vec::new(),
                    output: Vec::new(),
                },
            );
        }
        Ok(())
    };
    for (lifetime, run, _) in runs.into_values() {
        line(&mut lines, lifetime)?;
        lines.get_mut(&lifetime).unwrap().runs.push(run);
    }
    for (lifetime, output, _) in outputs.into_values() {
        line(&mut lines, lifetime)?;
        lines.get_mut(&lifetime).unwrap().output.push(output);
    }
    Ok(lines.into_values().collect())
}

/// A branch's published index, as a query reads it.
pub(super) struct Index<'a> {
    pages: Box<dyn Pages + 'a>,
    pub directory: Directory,
    published: Arc<Published>,
    /// The commit's bytes' identity, which a cache of what the index read is keyed by;
    /// none for fixture pages.
    pub publication: Option<[u64; 3]>,
    /// The store key such a cache is bound to.
    pub binding: Option<store::SourceKey>,
    /// The chunk table's pages read so far.
    table: HashMap<u64, Arc<Vec<(u64, i32, i32)>>>,
    opened: HashMap<u64, Opened>,
    section: String,
    /// Parts decoded since it opened.
    pub decoded: usize,
}

impl<'a> Index<'a> {
    /// The index the directory's last commit publishes.
    pub fn open(mut pages: Box<dyn Pages + 'a>, directory: Directory) -> io::Result<Self> {
        let published = Arc::new(Published::read(pages.as_mut(), &directory)?);
        Ok(Self::with(pages, directory, published, None))
    }

    /// The index `published` names, read from `pages`.
    pub fn with(
        pages: Box<dyn Pages + 'a>,
        directory: Directory,
        published: Arc<Published>,
        publication: Option<[u64; 3]>,
    ) -> Self {
        Self {
            pages,
            section: index::index_section(directory.branch),
            directory,
            published,
            publication,
            binding: None,
            table: HashMap::new(),
            opened: HashMap::new(),
            decoded: 0,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn identity(&self) -> &Identity {
        &self.published.identity
    }

    /// What the index publishes, for an index of the same publication read through other pages.
    pub fn published(&self) -> Arc<Published> {
        Arc::clone(&self.published)
    }

    /// The history a literal tag's view reads: none held is the absence's.
    pub fn tag_history(&self, tag: &str) -> u64 {
        self.published
            .identity
            .tags
            .iter()
            .find(|(held, _)| held == tag)
            .map_or(self.published.identity.absent, |(_, history)| *history)
    }

    pub fn native_history(&self, country: &NativeCountry) -> Option<u64> {
        self.published
            .identity
            .natives
            .iter()
            .find(|(native, _)| native == country)
            .map(|(_, history)| *history)
    }

    fn view(&self, history: u64) -> io::Result<&HistoryView> {
        self.published
            .histories
            .get(&history)
            .ok_or_else(|| crate::invalid("Missing production history"))
    }

    /// How many chunks the publication holds.
    pub fn len(&self) -> u64 {
        self.published.commit.chunks
    }

    /// The `at`th chunk, from its page of the chunk table or the commit's partial page.
    pub fn chunk(&mut self, at: u64) -> io::Result<Chunk> {
        let chunks = self.published.commit.chunks;
        let paged = chunks - self.published.commit.tail.len() as u64;
        if at >= chunks {
            return Err(crate::invalid("Missing production index chunk"));
        }
        let (range, first, last) = if at >= paged {
            self.published.commit.tail[(at - paged) as usize]
        } else {
            let page = at / index::CHUNK_PAGE as u64;
            let entries = match self.table.get(&page) {
                Some(entries) => Arc::clone(entries),
                None => {
                    let bytes = self
                        .pages
                        .read_at(&index::chunks_section(self.directory.branch), page)?
                        .ok_or_else(|| crate::invalid("Missing production chunk table page"))?;
                    let entries = Arc::new(index::decode_chunks(&bytes)?);
                    self.table.insert(page, Arc::clone(&entries));
                    entries
                }
            };
            entries[(at % index::CHUNK_PAGE as u64) as usize]
        };
        Ok(Chunk { range, first, last })
    }

    /// Every chunk, in order.
    #[cfg(any(test, feature = "test-support"))]
    pub fn chunks(&mut self) -> io::Result<Vec<Chunk>> {
        (0..self.len()).map(|at| self.chunk(at)).collect()
    }

    /// The first chunk for which `before` fails; it holds for a prefix, as chunk hours only
    /// grow.
    fn partition(&mut self, before: impl Fn(Chunk) -> bool) -> io::Result<u64> {
        let (mut low, mut high) = (0, self.len());
        while low < high {
            let middle = low + (high - low) / 2;
            if before(self.chunk(middle)?) {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        Ok(low)
    }

    /// The chunks a history's view reads: a fork reads its parent's before it began and its
    /// own from there.
    fn segments(&self, history: u64) -> io::Result<Segments> {
        let total = self.len();
        let mut segments = Vec::new();
        let (mut at, mut until) = (history, total);
        loop {
            if segments.len() > self.published.histories.len() {
                return Err(crate::invalid("Production histories form a cycle"));
            }
            let view = self.view(at)?;
            let from = view.parent.map_or(0, |(_, cut)| cut).min(until);
            segments.push((at, from, until));
            match view.parent {
                Some((parent, cut)) => {
                    at = parent;
                    until = cut.min(until);
                }
                None => break,
            }
        }
        segments.reverse();
        Ok(segments)
    }

    fn opened(&mut self, chunk: u64) -> io::Result<Opened> {
        if let Some(opened) = self.opened.get(&chunk) {
            return Ok(*opened);
        }
        let range = self.chunk(chunk)?.range;
        let opened = index::open(self.pages.as_mut(), &self.section, range)?;
        self.opened.insert(chunk, opened);
        Ok(opened)
    }

    fn part<T: product_codec::Product>(
        &mut self,
        chunk: u64,
        history: u64,
        part: Part,
        key: u64,
    ) -> io::Result<Option<T>> {
        let opened = self.opened(chunk)?;
        let found = index::part(
            self.pages.as_mut(),
            &self.section,
            opened,
            history,
            part,
            key,
        )?;
        self.decoded += usize::from(found.is_some());
        Ok(found)
    }

    fn parts(&mut self, chunk: u64, history: u64, part: Part) -> io::Result<Vec<index::Slice>> {
        let opened = self.opened(chunk)?;
        index::find_all(self.pages.as_mut(), &self.section, opened, history, part)
    }

    fn slice<T: product_codec::Product>(
        &mut self,
        chunk: u64,
        slice: index::Slice,
    ) -> io::Result<T> {
        let opened = self.opened(chunk)?;
        self.decoded += 1;
        index::read(self.pages.as_mut(), &self.section, opened, slice)
    }

    /// `owner`'s overview in `chunk`, if it holds one.
    fn overview_at(&mut self, chunk: u64, owner: u64) -> io::Result<Option<index::Overview>> {
        self.part(chunk, owner, Part::Overview, 0)
    }

    fn detail_at(&mut self, chunk: u64, owner: u64) -> io::Result<Option<index::Detail>> {
        self.part(chunk, owner, Part::Detail, 0)
    }

    fn closes_at(&mut self, chunk: u64, owner: u64) -> io::Result<Option<index::Closes>> {
        self.part(chunk, owner, Part::Closes, 0)
    }

    fn versions_part(&mut self, chunk: u64, owner: u64) -> io::Result<Option<index::Versions>> {
        self.part(chunk, owner, Part::Versions, 0)
    }

    /// The icons the census inputs held, from every chunk, in hour order.
    fn timeline(&mut self, cancelled: &AtomicBool) -> io::Result<Icons> {
        let mut icons = Icons::default();
        for chunk in 0..self.len() {
            crate::metadata::check_cancel(cancelled)?;
            if let Some(part) =
                self.part::<index::Timeline>(chunk, index::GLOBAL, Part::Timeline, 0)?
            {
                for (hour, variant, icon) in part.changes {
                    icons.changes.entry(variant).or_default().push((hour, icon));
                }
                icons.resets.extend(part.resets);
            }
        }
        Ok(icons)
    }

    /// The variants' versions and icon events of every chunk, folded in order.
    pub fn variants(&mut self, cancelled: &AtomicBool) -> io::Result<VariantFold> {
        let _naming = replay_frame::cpu::scope("production naming", Some("production-lines"));
        let mut versions: BTreeMap<u64, Vec<equipment::VariantVersion>> = BTreeMap::new();
        let mut icons = Vec::new();
        for chunk in 0..self.len() {
            crate::metadata::check_cancel(cancelled)?;
            if let Some(part) = self.part::<BTreeMap<u64, Vec<equipment::VariantVersion>>>(
                chunk,
                index::GLOBAL,
                Part::Variants,
                0,
            )? {
                for (id, next) in part {
                    let known = versions.entry(id).or_default();
                    for version in next {
                        match known
                            .last_mut()
                            .filter(|last| last.first_hour == version.first_hour)
                        {
                            Some(last) => *last = version,
                            None => known.push(version),
                        }
                    }
                }
            }
            if let Some(part) = self.part::<Vec<(u64, i32, IconChoice, bool)>>(
                chunk,
                index::GLOBAL,
                Part::IconEvents,
                0,
            )? {
                icons.extend(part);
            }
        }
        Ok(VariantFold {
            catalogue: VariantCatalogue::resumed(versions, self.published.commit.variants_hour),
            icons,
        })
    }

    /// The native country holding `tag` at `hour`, as the chunks' alias spans record it.
    pub fn country_at(&mut self, tag: &str, hour: i32) -> io::Result<Option<NativeCountry>> {
        for chunk in 0..self.len() {
            let Some(aliases) = self.part::<Vec<Alias>>(chunk, index::GLOBAL, Part::Aliases, 0)?
            else {
                continue;
            };
            if let Some(alias) = aliases.into_iter().find(|alias| {
                alias.tag == tag && alias.first_hour <= hour && hour <= alias.last_hour
            }) {
                return Ok(Some(alias.country));
            }
        }
        Ok(None)
    }

    /// The history's latest counts, which its last overview holds.
    pub fn counters(&mut self, history: u64) -> io::Result<index::Counters> {
        let Some(chunk) = self.view(history)?.overview else {
            return Ok(index::Counters::default());
        };
        let owner = owner(&self.segments(history)?, chunk)
            .map_err(|_| crate::invalid("Missing production overview"))?;
        Ok(self
            .overview_at(chunk, owner)?
            .ok_or_else(|| crate::invalid("Missing production overview"))?
            .counters)
    }

    pub fn overview(&mut self, history: u64, cancelled: &AtomicBool) -> io::Result<Overview> {
        let view = self.view(history)?.clone();
        let segments = self.segments(history)?;
        let mut overview = Overview::default();
        let mut lines: HashMap<u64, usize> = HashMap::new();
        let mut open: HashMap<u64, (usize, usize)> = HashMap::new();
        let timeline = self.timeline(cancelled)?;
        let mut icons: HashMap<u64, IconChoice> = HashMap::new();
        for (chunk, owner) in walk(&segments) {
            crate::metadata::check_cancel(cancelled)?;
            let Some(part) = self.overview_at(chunk, owner)? else {
                continue;
            };
            // Each census's icon of a variant it produced, where it differs from the last.
            let mut produced = part.produced.clone();
            produced.sort_unstable();
            for (variant, first, last) in produced {
                let Some(held) = timeline.changes.get(&variant) else {
                    continue;
                };
                // The run's first census, then the first after each choice made within it.
                let mut hours = vec![first];
                let within = held.partition_point(|(changed, _)| *changed <= first);
                for &(changed, _) in held[within..]
                    .iter()
                    .take_while(|(changed, _)| *changed <= last)
                {
                    if let Some(&read) = part
                        .reads
                        .get(part.reads.partition_point(|read| *read < changed))
                        && read <= last
                    {
                        hours.push(read);
                    }
                }
                hours.dedup();
                for hour in hours {
                    let Some(icon) = timeline.at(held, hour) else {
                        continue;
                    };
                    if icons.get(&variant) != Some(icon) {
                        icons.insert(variant, icon.clone());
                        overview
                            .icons
                            .entry(variant)
                            .or_default()
                            .push((hour, icon.clone()));
                    }
                }
            }
            for life in part.lives {
                lines.insert(life.id, overview.lines.len());
                overview.lines.push(OverviewLine {
                    lifetime: life.id,
                    key: format!("{}#{}", life.handle, life.appearance),
                    handle: life.handle,
                    kind: life.kind,
                    steps: Vec::new(),
                    final_output: None,
                    ended_by: None,
                    before_recording: life.before_recording,
                    between_reads: life.between_reads,
                    variants: life.variants,
                });
            }
            let line = |lifetime: u64| {
                lines
                    .get(&lifetime)
                    .copied()
                    .ok_or_else(|| crate::invalid("Production event names no lifetime"))
            };
            for step in part.steps {
                let at = line(step.lifetime)?;
                if !step.closed {
                    open.insert(step.id, (at, overview.lines[at].steps.len()));
                }
                overview.lines[at].steps.push(step);
            }
            for (id, last) in part.step_closes {
                if let Some((at, step)) = open.remove(&id) {
                    let step = &mut overview.lines[at].steps[step];
                    step.last_hour = last;
                    step.closed = true;
                }
            }
            for (lifetime, hour) in part.ends {
                overview.lines[line(lifetime)?].ended_by = Some(hour);
            }
            for (lifetime, output) in part.finals {
                overview.lines[line(lifetime)?].final_output = Some(output);
            }
            overview.reads.extend(part.reads);
            for (to, spans) in [
                (&mut overview.line_hours, part.line_hours),
                (&mut overview.output_hours, part.output_hours),
                (&mut overview.organisation_hours, part.organisation_hours),
            ] {
                for span in spans {
                    mark_span(to, span.first_hour, span.last_hour);
                }
            }
            if part.wakes.windows(2).any(|pair| pair[0].at > pair[1].at)
                || part
                    .wakes
                    .last()
                    .is_some_and(|wake| wake.at > part.unread_output.len() as u64)
            {
                return Err(crate::invalid("A production wake lies outside its spans"));
            }
            let mut wakes = part.wakes.iter().peekable();
            for (at, span) in part.unread_output.iter().enumerate() {
                while let Some(wake) = wakes.next_if(|wake| wake.at == at as u64) {
                    self.fold_absence(&mut overview.unread_output, wake, cancelled)?;
                }
                mark_span(&mut overview.unread_output, span.first_hour, span.last_hour);
            }
            for wake in wakes {
                self.fold_absence(&mut overview.unread_output, wake, cancelled)?;
            }
            overview.used.extend(part.output_variants);
            for (id, raw) in part.names {
                overview.organisations.entry(id).or_default().push(raw);
            }
            overview.counters = part.counters;
        }
        for (at, step) in open.into_values() {
            let step = &mut overview.lines[at].steps[step];
            step.last_hour = view.through.unwrap_or(step.last_hour);
        }
        overview.reads.dedup();
        for line in &overview.lines {
            overview
                .used
                .extend(line.steps.iter().map(|step| step.variant));
            overview.used.extend(line.variants.iter().copied());
            overview.used.extend(
                line.final_output
                    .iter()
                    .flat_map(|output| output.parts.iter().map(|part| part.variant)),
            );
        }
        if let Some(sleep) = view.sleep {
            let counters = self.counters(self.published.identity.absent)?;
            overview.counters.unread_output_reads +=
                counters.unread_output_reads.saturating_sub(sleep.reads);
            overview.counters.unreadable_stretches += counters
                .unreadable_stretches
                .saturating_sub(sleep.stretches);
            let wake = index::Wake {
                at: 0,
                from: sleep.spans,
                to: self.published.commit.spans,
                hour: sleep.hour,
            };
            self.fold_absence(&mut overview.unread_output, &wake, cancelled)?;
        }
        Ok(overview)
    }

    /// Folds into `spans` what `wake` names: the absence's unread spans from its `from`th to
    /// before its `to`th that end after its hour, each from the hour after. They are read a
    /// page at a time from `production-absence`, and from the commit past its last full page.
    fn fold_absence(
        &mut self,
        spans: &mut Vec<HourSpan>,
        wake: &index::Wake,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let published = Arc::clone(&self.published);
        let commit = &published.commit;
        if wake.from > wake.to || wake.to > commit.spans {
            return Err(crate::invalid(
                "A production wake names unread spans the absence never recorded",
            ));
        }
        let mut add = |span: &HourSpan| {
            if span.last_hour > wake.hour {
                mark_span(
                    spans,
                    span.first_hour.max(wake.hour.saturating_add(1)),
                    span.last_hour,
                );
            }
        };
        let page_spans = index::SPAN_PAGE as u64;
        let paged = commit.spans - commit.span_tail.len() as u64;
        let mut at = wake.from;
        while at < wake.to.min(paged) {
            crate::metadata::check_cancel(cancelled)?;
            let page = at / page_spans;
            let bytes = self
                .pages
                .read_at(&index::absence_section(self.directory.branch), page)?
                .ok_or_else(|| crate::invalid("Missing production absence page"))?;
            let entries = index::decode_spans(&bytes)?;
            let start = (at - page * page_spans) as usize;
            let end = (wake.to - page * page_spans).min(page_spans) as usize;
            for span in &entries[start..end] {
                add(span);
            }
            at = (page + 1) * page_spans;
        }
        if at < wake.to {
            let start = (at - paged) as usize;
            let end = (wake.to - paged) as usize;
            for span in &commit.span_tail[start..end] {
                add(span);
            }
        }
        Ok(())
    }

    /// Every line's runs and output reads that hold any hour from `first` to `last`, whole,
    /// with the design and organisation versions they need.
    pub fn window(
        &mut self,
        history: u64,
        first: i32,
        last: i32,
        cancelled: &AtomicBool,
    ) -> io::Result<Window> {
        self.window_of(history, first, last, false, cancelled)
    }

    /// The Versions part a view reads at `chunk`, which must hold one.
    fn versions_at(&mut self, segments: &Segments, chunk: u64) -> io::Result<index::Versions> {
        let owner = owner(segments, chunk)?;
        self.versions_part(chunk, owner)?
            .ok_or_else(|| {
                crate::invalid("A production version anchor names a chunk without versions")
            })
    }

    /// How chunk `chunk` carries interval `id` (a run, or an output read where `output`):
    /// its end and whether it closed there, or none where it does not carry it.
    fn carried(
        &mut self,
        segments: &Segments,
        chunk: u64,
        id: u64,
        output: bool,
        closes: &mut HashMap<u64, Option<Arc<index::Closes>>>,
    ) -> io::Result<Option<(i32, bool)>> {
        let part = match closes.get(&chunk) {
            Some(part) => part.clone(),
            None => {
                let owner = owner(segments, chunk)?;
                let part = self.closes_at(chunk, owner)?.map(Arc::new);
                closes.insert(chunk, part.clone());
                part
            }
        };
        Ok(part.and_then(|part| {
            let list = if output { &part.outputs } else { &part.runs };
            list.iter()
                .find(|(found, _, _)| *found == id)
                .map(|&(_, end, closed)| (end, closed))
        }))
    }

    /// Where each interval still open after chunk `high` closes: in the first later chunk
    /// that does not carry it open, found by bisection, as a history carries every open
    /// interval into each chunk it writes and writes each chunk while it holds one. One no
    /// later chunk closes is absent.
    fn ends(
        &mut self,
        segments: &Segments,
        high: u64,
        intervals: &BTreeSet<u64>,
        output: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<BTreeMap<u64, i32>> {
        let total = self.len();
        let mut closes = HashMap::new();
        let mut ends = BTreeMap::new();
        for &id in intervals {
            crate::metadata::check_cancel(cancelled)?;
            let (mut low, mut until) = (high + 1, total);
            while low < until {
                let middle = low + (until - low) / 2;
                let carried = self.carried(segments, middle, id, output, &mut closes)?;
                if carried.is_none_or(|(_, closed)| closed) {
                    until = middle;
                } else {
                    low = middle + 1;
                }
            }
            if low < total {
                match self.carried(segments, low, id, output, &mut closes)? {
                    Some((end, true)) => {
                        ends.insert(id, end);
                    }
                    _ => {
                        return Err(crate::invalid(
                            "The production index does not carry an open interval",
                        ));
                    }
                }
            }
        }
        Ok(ends)
    }

    /// The first version of `variant` pushed after chunk `high` in a chunk that begins by
    /// `reach`. Each such chunk carries a run of it, so names where its last version was
    /// pushed; the first to name one after `high` follows the chunk that pushed it.
    fn next_design(
        &mut self,
        segments: &Segments,
        high: u64,
        variant: u64,
        reach: i32,
    ) -> io::Result<Option<DesignRead>> {
        let end = self.partition(|chunk| chunk.first <= reach)?;
        if end <= high + 1 {
            return Ok(None);
        }
        let (mut low, mut until) = (high + 2, end);
        while low < until {
            let middle = low + (until - low) / 2;
            let anchor = self
                .versions_at(segments, middle)?
                .design_anchors
                .get(&variant)
                .copied()
                .ok_or_else(|| {
                    crate::invalid("A production chunk carrying a run does not anchor its design")
                })?;
            if anchor.is_some_and(|at| at > high) {
                until = middle;
            } else {
                low = middle + 1;
            }
        }
        let at = low - 1;
        Ok(self
            .versions_part(at, owner(segments, at)?)?
            .and_then(|part| {
                part.designs
                    .iter()
                    .find(|(id, _)| *id == variant)
                    .map(|(_, read)| *read)
            }))
    }

    /// [`Self::window`], with every design and organisation version its chunks hold when
    /// `every`.
    fn window_of(
        &mut self,
        history: u64,
        first: i32,
        last: i32,
        every: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<Window> {
        let horizon = self.view(history)?.through;
        let segments = self.segments(history)?;
        let low = self.partition(|chunk| chunk.last < first)?;
        let end = self.partition(|chunk| chunk.first <= last)?;
        if low >= end {
            return Ok(Window::default());
        }
        let high = end - 1;
        let mut runs: BTreeMap<u64, (u64, RawRun, bool)> = BTreeMap::new();
        let mut outputs: BTreeMap<u64, (u64, RawOutput, bool)> = BTreeMap::new();
        let mut lives: BTreeMap<u64, (u64, u32)> = BTreeMap::new();
        let mut versions: Vec<index::Versions> = Vec::new();
        for chunk in low..=high {
            crate::metadata::check_cancel(cancelled)?;
            let owner = owner(&segments, chunk)?;
            if let Some(detail) = self.detail_at(chunk, owner)? {
                detail.carried_runs.checked()?;
                detail.runs.checked()?;
                lives.extend(detail.lives);
                for list in [&detail.carried_runs, &detail.runs] {
                    for at in 0..list.len() {
                        let entry = runs
                            .entry(list.id[at])
                            .or_insert_with(|| (list.lifetime[at], list.run(at), false));
                        if list.closed[at] {
                            entry.1.last_hour = list.last[at];
                            entry.2 = true;
                        }
                    }
                }
                for record in detail.carried_outputs.into_iter().chain(detail.outputs) {
                    let entry = outputs
                        .entry(record.id)
                        .or_insert_with(|| (record.lifetime, record.output.clone(), false));
                    if record.closed {
                        entry.1.last_hour = record.output.last_hour;
                        entry.2 = true;
                    }
                }
            }
            if let Some(part) = self.versions_part(chunk, owner)? {
                versions.push(part);
            }
        }
        let overlaps = |from: i32, to: i32, closed: bool| from <= last && (!closed || to >= first);
        runs.retain(|_, (_, run, closed)| overlaps(run.first_hour, run.last_hour, *closed));
        outputs.retain(|_, (_, output, closed)| {
            overlaps(output.first_hour, output.last_hour, *closed)
        });
        let open_runs: BTreeSet<u64> = runs
            .iter()
            .filter(|(_, (_, _, closed))| !closed)
            .map(|(id, _)| *id)
            .collect();
        let open_outputs: BTreeSet<u64> = outputs
            .iter()
            .filter(|(_, (_, _, closed))| !closed)
            .map(|(id, _)| *id)
            .collect();
        let run_ends = self.ends(&segments, high, &open_runs, false, cancelled)?;
        let output_ends = self.ends(&segments, high, &open_outputs, true, cancelled)?;
        for id in open_runs {
            let run = &mut runs.get_mut(&id).unwrap().1;
            run.last_hour = match run_ends.get(&id) {
                Some(end) => *end,
                None => horizon.unwrap_or(run.last_hour),
            };
        }
        for id in open_outputs {
            let output = &mut outputs.get_mut(&id).unwrap().1;
            output.last_hour = match output_ends.get(&id) {
                Some(end) => *end,
                None => horizon.unwrap_or(output.last_hour),
            };
        }
        let mut window = Window::default();
        for part in &versions {
            for (variant, read) in &part.designs {
                window.designs.entry(*variant).or_default().push(*read);
            }
            for (id, raw) in &part.organisations {
                window
                    .organisations
                    .entry(*id)
                    .or_default()
                    .push(raw.clone());
            }
        }
        let mut reaches: BTreeMap<u64, i32> = BTreeMap::new();
        let mut demanded = BTreeSet::new();
        for (_, run, _) in runs.values() {
            let reach = reaches.entry(run.variant).or_insert(run.last_hour);
            *reach = (*reach).max(run.last_hour);
            if run.mio != 0 {
                demanded.insert(run.mio);
            }
        }
        if every {
            return Ok(Window {
                lines: window_lines(runs, outputs, &lives)?,
                ..window
            });
        }
        window
            .designs
            .retain(|variant, _| reaches.contains_key(variant));
        window.organisations.retain(|id, _| demanded.contains(id));
        let high_last = self.chunk(high)?.last;
        // A version before the window still holds at its start: the first chunk of the window
        // naming the variant says where it was pushed. One after it ends a run's last overlay.
        for (&variant, &reach) in &reaches {
            if window
                .designs
                .get(&variant)
                .is_none_or(|list| list.iter().all(|read| read.first_hour > first))
            {
                let anchor = versions
                    .iter()
                    .find_map(|part| part.design_anchors.get(&variant).copied())
                    .ok_or_else(|| {
                        crate::invalid("A production window does not anchor a design")
                    })?;
                if let Some(at) = anchor {
                    let read = self
                        .versions_at(&segments, at)?
                        .designs
                        .iter()
                        .rev()
                        .find(|(id, _)| *id == variant)
                        .map(|(_, read)| *read)
                        .ok_or_else(|| {
                            crate::invalid("A production design anchor names no version")
                        })?;
                    window.designs.entry(variant).or_default().insert(0, read);
                }
            }
            if reach > high_last
                && let Some(read) = self.next_design(&segments, high, variant, reach)?
            {
                window.designs.entry(variant).or_default().push(read);
            }
        }
        for &id in &demanded {
            if window
                .organisations
                .get(&id)
                .is_none_or(|list| list.iter().all(|raw| raw.first_hour > first))
            {
                let anchor = versions
                    .iter()
                    .find_map(|part| part.organisation_anchors.get(&id).copied())
                    .ok_or_else(|| {
                        crate::invalid("A production window does not anchor an organisation")
                    })?;
                if let Some(at) = anchor {
                    let raw = self
                        .versions_at(&segments, at)?
                        .organisations
                        .into_iter()
                        .rev()
                        .find(|(found, _)| *found == id)
                        .map(|(_, raw)| raw)
                        .ok_or_else(|| {
                            crate::invalid("A production organisation anchor names no version")
                        })?;
                    window.organisations.entry(id).or_default().insert(0, raw);
                }
            }
        }
        window.lines = window_lines(runs, outputs, &lives)?;
        Ok(window)
    }

    /// Each lifetime's modifier reads in order (only `lifetimes`', or every one), each with
    /// its country's row shared by the reads that read it unchanged.
    pub fn modifiers(
        &mut self,
        history: u64,
        lifetimes: Option<&BTreeSet<u64>>,
        localisation: Option<Arc<installed_content_localisation::localisation::Localisation>>,
        language: Arc<str>,
        cancelled: &AtomicBool,
    ) -> io::Result<BTreeMap<u64, Vec<breakdown::RawModifierRun>>> {
        let horizon = self.view(history)?.through;
        let segments = self.segments(history)?;
        let mut runs: BTreeMap<u64, Vec<breakdown::RawModifierRun>> = BTreeMap::new();
        let mut open: BTreeMap<u64, bool> = BTreeMap::new();
        let mut operands: BTreeMap<u64, replay_data::rows::ProductionLineModifiersRow> =
            BTreeMap::new();
        let mut last_country: Option<Arc<CountryProductionModifiersRow>> = None;
        for (chunk, owner) in walk(&segments) {
            crate::metadata::check_cancel(cancelled)?;
            let holder = owner;
            // Each part with its lifetime.
            let parts: Vec<(u64, index::Slice)> = match lifetimes {
                Some(lifetimes) => {
                    let mut found = Vec::new();
                    for &lifetime in lifetimes {
                        let opened = self.opened(chunk)?;
                        if let Some(slice) = index::find(
                            self.pages.as_mut(),
                            &self.section,
                            opened,
                            holder,
                            Part::Modifiers,
                            lifetime,
                        )? {
                            found.push((lifetime, slice));
                        }
                    }
                    found
                }
                None => self
                    .parts(chunk, holder, Part::Modifiers)?
                    .into_iter()
                    .map(|slice| (slice.key, slice))
                    .collect(),
            };
            let rows = match (lifetimes, parts.is_empty()) {
                (Some(_), true) => continue,
                _ => self.part::<Vec<Vec<u8>>>(chunk, holder, Part::Countries, 0)?,
            };
            let mut countries = Vec::new();
            for bytes in rows.unwrap_or_default() {
                let row = index::decode_country(&bytes)?;
                let shared = match &last_country {
                    Some(last) if **last == row => Arc::clone(last),
                    _ => Arc::new(row),
                };
                last_country = Some(Arc::clone(&shared));
                countries.push(shared);
            }
            for (lifetime, slice) in parts {
                let stored: index::StoredModifiers = self.slice(chunk, slice)?;
                let modifiers = stored.records()?;
                let reads = runs.entry(lifetime).or_default();
                if let Some(end) = modifiers.carried_last
                    && open.get(&lifetime) == Some(&true)
                    && let Some(read) = reads.last_mut()
                {
                    read.last_hour = end;
                    open.insert(lifetime, false);
                }
                for record in modifiers.records {
                    let line = match record.line {
                        Some(line) => {
                            operands.insert(lifetime, line.clone());
                            line
                        }
                        None => operands.get(&lifetime).cloned().ok_or_else(|| {
                            crate::invalid("Production modifier read repeats none")
                        })?,
                    };
                    let country =
                        countries
                            .get(record.country as usize)
                            .cloned()
                            .ok_or_else(|| {
                                crate::invalid("Production modifier read names no country")
                            })?;
                    open.insert(lifetime, !record.closed);
                    reads.push(breakdown::RawModifierRun {
                        first_hour: record.first_hour,
                        last_hour: record.last_hour,
                        country,
                        line,
                        run: record.run,
                        organisation: record.organisation,
                        localisation: localisation.clone(),
                        recorded_language: Arc::clone(&language),
                    });
                }
            }
        }
        for (lifetime, still) in open {
            if still && let Some(read) = runs.get_mut(&lifetime).and_then(|reads| reads.last_mut())
            {
                read.last_hour = horizon.unwrap_or(read.last_hour);
            }
        }
        runs.retain(|_, reads| !reads.is_empty());
        Ok(runs)
    }

    /// A history's whole branch as one scan: what the old pages assembled, for tests to
    /// compare with the independent reference.
    #[cfg(any(test, feature = "test-support"))]
    pub fn scanned(
        &mut self,
        history: u64,
        localisation: Option<Arc<installed_content_localisation::localisation::Localisation>>,
        language: Arc<str>,
        cancelled: &AtomicBool,
    ) -> io::Result<ScannedLines> {
        let overview = self.overview(history, cancelled)?;
        let window = self.window_of(history, i32::MIN, i32::MAX, true, cancelled)?;
        let modifiers = self.modifiers(history, None, localisation, language, cancelled)?;
        let fold = self.variants(cancelled)?;
        let mut reads: BTreeMap<u64, WindowLine> = window
            .lines
            .into_iter()
            .map(|line| (line.lifetime, line))
            .collect();
        let mut keys = BTreeMap::new();
        let lines = overview
            .lines
            .into_iter()
            .map(|line| {
                keys.insert(line.lifetime, line.key.clone());
                let read = reads.remove(&line.lifetime);
                ScannedLine {
                    key: line.key,
                    handle: line.handle,
                    kind: line.kind,
                    runs: read
                        .as_ref()
                        .map_or_else(Vec::new, |read| read.runs.clone()),
                    output: read.map_or_else(Vec::new, |read| read.output),
                    final_output: line.final_output,
                    ended_by: line.ended_by,
                    before_recording: line.before_recording,
                    between_reads: line.between_reads,
                    variants: line.variants,
                }
            })
            .collect();
        let mut icons = overview.icons;
        merge_icons(&mut icons, &fold.icons, cancelled)?;
        let mut organisations: BTreeMap<u64, Vec<RawOrganisation>> = BTreeMap::new();
        for (id, versions) in window.organisations {
            organisations.insert(id, versions);
        }
        Ok(ScannedLines {
            designs: window.designs,
            lines,
            modifiers: modifiers
                .into_iter()
                .filter_map(|(lifetime, reads)| Some((keys.get(&lifetime)?.clone(), reads)))
                .collect(),
            modifier_layout_refusals: overview.counters.refusals,
            variants: fold.catalogue,
            icons,
            organisations,
            reads: overview.reads,
            output_hours: overview.output_hours,
            line_hours: overview.line_hours,
            organisation_hours: overview.organisation_hours,
            unknown_kinds: overview.counters.unknown_kinds,
            unlisted_output: overview.counters.unlisted_output,
            unread_output_reads: overview.counters.unread_output_reads,
            unreadable_stretches: overview.counters.unreadable_stretches,
            unlinked_rows: overview.counters.unlinked_rows,
            unread_output: overview.unread_output,
        })
    }
}
