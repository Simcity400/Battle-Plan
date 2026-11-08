//! Prepared block records reuse the census, peak helpers and reply builders.
//! A block pages out the battles, open battles' runs and reads, and warnings
//! before its ranking interval; only a finalizer whose coverage reaches back
//! past them redraws a window from pages, a battle's slice at a time.
use dashboard_base::products::built::Built;
use super::*;
use dashboard_base::prepared::series;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::io::Read as _;

/// A battle a block's publication paged out.
#[derive(Serialize, Deserialize)]
struct Paged {
    at: usize,
    /// The combat whose newest live battle this was, which a delayed finalizer may still end.
    combat: Option<u64>,
    episode: Episode,
}

/// An open battle's loss reads paged out before those it holds, and the page
/// of those before them.
#[derive(replay_spool::Spooled)]
struct Earlier {
    losses: Vec<losses::LossRead>,
    /// The [`Course`] page of the runs paged out with these reads, and the reads wired.
    course: u64,
    earlier: Option<u64>,
    /// The hour this page and those before it hold every run that ended before.
    before: i32,
    /// The peaks of every run in this page and those before it.
    peaks: [Option<LandBattlePeak>; 3],
}

/// The runs an [`Earlier`] page paged out and its reads as the course reply
/// sends them, wired on from those before them; and the page of those before.
#[derive(replay_spool::Spooled)]
struct Course {
    runs: Vec<LandBattleRun>,
    losses: Vec<LandBattleCourseRead>,
    earlier: Option<u64>,
}

/// Where a battle's course reply stands after the reads it paged out, so its
/// course page holds only what it keeps.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Carried {
    /// Each side's variants at the last paged-out read ([`losses::LossRead::wire`]).
    previous: [Option<Vec<(usize, i64)>>; 2],
    /// The branch designs the paged-out reads named, in the reply's order.
    designs: Vec<usize>,
    /// The newest [`Course`] page.
    course: Option<u64>,
}

/// A battle's course page: the reply of the runs and reads it holds, after
/// those its [`Course`] pages hold from `history` back.
#[derive(Serialize, Deserialize)]
struct CoursePage {
    history: Option<u64>,
    reply: LandBattleCourseReply,
}

/// An earlier window as a publication redraws it: every battle that may fall
/// in it, by ordinal, the warnings dated in it and the read spans about it.
struct Drawn<'a> {
    episodes: BTreeMap<usize, Cow<'a, Episode>>,
    warnings: warnings::Warnings,
    observed: Vec<(i32, i32)>,
}

#[cfg(test)]
thread_local! {
    /// Keeps every battle in memory, so tests can compare paged preparation with it.
    pub(super) static KEEP_ALL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn read_page<T: DeserializeOwned>(
    writer: &mut store::Writer,
    section: &str,
    page: usize,
) -> io::Result<T> {
    let mut range = writer
        .read_at(section, page as u64)?
        .ok_or_else(|| io::Error::other(format!("Prepared {section} page {page} is missing")))?;
    let mut bytes = Vec::new();
    range.read_to_end(&mut bytes)?;
    Ok(serde_json::from_slice(&series::decompress(&bytes)?)?)
}

fn unspool<T: replay_spool::Spooled>(compressed: &[u8]) -> io::Result<T> {
    let bytes = series::decompress(compressed)?;
    let mut restored = replay_spool::Restored::default();
    let mut reader = replay_spool::Reader::new(&bytes, &mut restored);
    let page = T::unspool(&mut reader)?;
    reader.finish()?;
    Ok(page)
}

fn spool<T: replay_spool::Spooled>(
    writer: &mut store::Writer,
    section: &str,
    value: &T,
    scratch: &mut Vec<u8>,
) -> io::Result<u64> {
    scratch.clear();
    {
        let mut shared = replay_spool::Shared::default();
        value.spool(&mut replay_spool::Writer::new(scratch, &mut shared))?;
    }
    writer.write(section, series::compress(scratch, 3)?.as_slice())
}

fn spooled_page<T: replay_spool::Spooled>(
    writer: &mut store::Writer,
    section: &str,
    page: u64,
) -> io::Result<T> {
    let mut range = writer
        .read_at(section, page)?
        .ok_or_else(|| io::Error::other(format!("Prepared {section} page {page} is missing")))?;
    let mut bytes = Vec::new();
    range.read_to_end(&mut bytes)?;
    unspool(&bytes)
}

/// Adds `item`'s JSON to a reply of `bytes` whose array holds `held` items,
/// refusing the reply once it passes its limit.
fn grow(bytes: &mut usize, held: &mut usize, item: &impl Serialize) -> io::Result<()> {
    let len = json_len(item, MAX_PRODUCT_REPLY_BYTES).ok_or_else(too_long)?;
    *bytes = bytes.saturating_add(len + usize::from(*held > 0));
    *held += 1;
    if *bytes > MAX_PRODUCT_REPLY_BYTES {
        return Err(too_long());
    }
    Ok(())
}

/// `course`'s whole reply with `warnings`: the runs and wired reads of its
/// [`Course`] pages, oldest first, ahead of those its page holds. Read newest
/// page first, it is refused as soon as it would pass the reply limit.
fn stored_course(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    course: CoursePage,
    warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<LandBattleCourseReply> {
    let CoursePage { history, mut reply } = course;
    reply.content_warnings = warnings;
    if history.is_none() {
        return bounded(reply);
    }
    // Counted exactly as `bounded` counts the whole reply.
    let mut bytes = json_len(&reply, MAX_PRODUCT_REPLY_BYTES).ok_or_else(too_long)?;
    let mut held = (reply.battle.runs.len(), reply.battle.losses.len());
    let section = format!("land-battle-course-history-{branch}");
    let mut pages = Vec::new();
    let mut next = history;
    while let Some(page) = next {
        check_cancel(cancelled)?;
        let mut range = store.read(key, &section, page)?.ok_or_else(|| {
            dashboard_core::invalid("Prepared land battle course history is missing")
        })?;
        let mut compressed = Vec::new();
        range.read_to_end(&mut compressed)?;
        let part: Course = unspool(&compressed)?;
        for run in &part.runs {
            grow(&mut bytes, &mut held.0, run)?;
        }
        for read in &part.losses {
            grow(&mut bytes, &mut held.1, read)?;
        }
        next = part.earlier;
        pages.push(part);
    }
    let mut runs = Vec::with_capacity(held.0);
    let mut losses = Vec::with_capacity(held.1);
    for part in pages.into_iter().rev() {
        runs.extend(part.runs);
        losses.extend(part.losses);
    }
    runs.append(&mut reply.battle.runs);
    losses.append(&mut reply.battle.losses);
    reply.battle.runs = runs;
    reply.battle.losses = losses;
    Ok(reply)
}

/// What [`slice`] has taken of a battle's segments, newest first.
struct Slicing {
    first: i32,
    last: i32,
    carry: i32,
    runs: Vec<LandBattleRun>,
    before: Vec<LandBattleRun>,
    /// The reads in the hours and the one carried into them, newest appended first.
    reads: Vec<losses::LossRead>,
    /// Whether `reads` holds the newest-appended read at or before `carry`.
    carried: bool,
}
impl Slicing {
    /// Takes what a redraw needs of a segment older than those taken before:
    /// its runs too when no page's earlier peaks hold them yet.
    fn take(&mut self, runs: &[LandBattleRun], reads: &[losses::LossRead], with_runs: bool) {
        if with_runs {
            for run in runs.iter().rev() {
                if run.last_hour < self.first {
                    self.before.push(run.clone());
                } else if run.first_hour <= self.last {
                    self.runs.push(run.clone());
                }
            }
        }
        if !self.carried {
            // A finalizer's read can be dated before reads appended ahead of it.
            for read in reads.iter().rev() {
                if read.hour > self.carry {
                    if read.hour <= self.last {
                        self.reads.push(read.clone());
                    }
                } else if !self.carried {
                    self.carried = true;
                    self.reads.push(read.clone());
                }
            }
        }
    }
}

/// What a redraw of hours `first` through `last` reads of `episode`: its runs
/// reaching them and its last run, the loss read carried into them and those in
/// them, and the peaks of its runs before them. Its history is read newest page
/// first, a page at a time, back only as far as those need.
fn slice<'a>(
    writer: &mut store::Writer,
    branch: u64,
    episode: &'a Episode,
    (first, last): (i32, i32),
    cancelled: &AtomicBool,
) -> io::Result<Cow<'a, Episode>> {
    let Some(mut page) = episode.history else {
        return Ok(Cow::Borrowed(episode));
    };
    let last_run = episode.runs.last().expect("observed episode");
    let mut slicing = Slicing {
        first,
        last,
        carry: first.min(last_run.last_hour),
        runs: Vec::new(),
        before: Vec::new(),
        reads: Vec::new(),
        carried: false,
    };
    slicing.take(&episode.runs, &episode.losses, true);
    let mut peaks = None;
    loop {
        check_cancel(cancelled)?;
        let part: Earlier = spooled_page(writer, &format!("land-battle-history-{branch}"), page)?;
        // Every run this page and those before it hold ended before `first`.
        let held = part.before <= first;
        let with_runs = peaks.is_none() && !held;
        let runs = if with_runs {
            let section = format!("land-battle-course-history-{branch}");
            spooled_page::<Course>(writer, &section, part.course)?.runs
        } else {
            Vec::new()
        };
        slicing.take(&runs, &part.losses, with_runs);
        if held && peaks.is_none() {
            peaks = Some(part.peaks.clone());
        }
        match part.earlier {
            Some(next) if peaks.is_none() || !slicing.carried => page = next,
            _ => break,
        }
    }
    let Slicing {
        mut runs,
        mut before,
        mut reads,
        ..
    } = slicing;
    let mut slice = episode.clone();
    slice.prior_peaks = peaks.unwrap_or([None, None, None]);
    before.reverse();
    slice.fold_peaks(&before);
    runs.reverse();
    if runs
        .last()
        .is_none_or(|run| run.first_hour != last_run.first_hour)
    {
        runs.push(last_run.clone());
    }
    slice.runs = runs;
    reads.reverse();
    slice.losses = reads;
    slice.history = None;
    slice.carried = None;
    Ok(Cow::Owned(slice))
}

impl Census {
    pub fn new(format: replay_data::format::FormatVersion) -> Self {
        Self {
            format,
            ..Self::default()
        }
    }
    /// Names designs from `source`'s content, as the row census's tests prepare it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn configure(&mut self, source: &Source, context: &LoadContext) {
        if self.configured {
            return;
        }
        self.configured = true;
        self.format = source.replay_ref().format;
        let (catalogue, warning) = battle_content(source, context);
        self.catalogue = catalogue;
        self.content_warnings = warning.into_iter().collect();
    }
    /// The first hour of window `index`'s ranking interval.
    fn interval_first(&self, windows: &[BattleWindow], index: usize) -> i32 {
        if index == 0 {
            self.first_start
                .unwrap_or(windows[0].first_hour)
                .min(windows[0].first_hour)
        } else {
            windows[index - 1]
                .last_hour
                .checked_add(1)
                .unwrap_or(windows[index].first_hour)
        }
    }
    /// The battles and warnings of `window`, from `drawn` or else from memory.
    fn window_records(&self, window: &BattleWindow, drawn: Option<&Drawn>) -> Self {
        let first = window.first_hour;
        let last = window.last_hour;
        let (candidates, warnings): (Box<dyn Iterator<Item = (usize, &Episode)> + '_>, _) =
            match drawn {
                Some(drawn) => (
                    Box::new(
                        drawn
                            .episodes
                            .iter()
                            .map(|(at, episode)| (*at, episode.as_ref())),
                    ),
                    &drawn.warnings,
                ),
                None => (
                    Box::new(self.episodes.iter().map(|(at, episode)| (*at, episode))),
                    &self.warnings,
                ),
            };
        let episodes = candidates
            .filter_map(|(at, episode)| {
                let episode = self.effective_episode(episode);
                let end_in_window = episode
                    .end
                    .as_ref()
                    .is_some_and(|(hour, _)| *hour >= first && *hour <= last);
                let mut runs: Vec<_> = episode
                    .runs
                    .iter()
                    .filter(|run| run.first_hour <= last && run.last_hour >= first)
                    .map(|run| run.clipped(run.first_hour.max(first), run.last_hour.min(last)))
                    .collect();
                if runs.is_empty() && end_in_window {
                    let run = episode.runs.last().expect("observed episode");
                    runs.push(run.clipped(run.last_hour, run.last_hour));
                }
                if runs.is_empty() {
                    return None;
                }
                let loss_first = first.min(runs[0].first_hour);
                // Append order: a backdated finalizer's read supersedes reads appended before it.
                let carried = episode
                    .losses
                    .iter()
                    .rposition(|read| read.hour <= loss_first);
                // Built from its parts: the battle's whole runs and reads are never copied.
                let row = Episode {
                    id: episode.id.clone(),
                    province: episode.province,
                    start: episode.start.clone(),
                    end: episode.end.clone(),
                    counter_base: episode.counter_base,
                    runs,
                    losses: episode
                        .losses
                        .iter()
                        .enumerate()
                        .filter(|&(at, read)| {
                            Some(at) == carried || read.hour > loss_first && read.hour <= last
                        })
                        .map(|(_, read)| read.clone())
                        .collect(),
                    result: episode.result.clone(),
                    loss_inputs: None,
                    prior_peaks: std::array::from_fn(|index| {
                        first.checked_sub(1).and_then(|hour| {
                            if index == 0 {
                                episode.peak_before(hour)
                            } else {
                                episode.side_peak_before(hour, index - 1)
                            }
                        })
                    }),
                    history: None,
                    carried: None,
                };
                Some((at, row))
            })
            .collect();
        let spans: Vec<_> = match drawn {
            Some(drawn) => drawn.observed.clone(),
            None => self.observed_spans().collect(),
        };
        Self {
            episodes,
            observed: spans
                .iter()
                .copied()
                .filter(|(from, to)| *from <= last && *to >= first)
                .chain(spans.iter().rev().copied().find(|(_, to)| *to < first))
                .collect(),
            warnings: self.counts_in(warnings, first, last),
            ..Self::default()
        }
    }
    /// Window `index`'s ranking `interval` when it falls before the paged hours: its
    /// battles paged since, under those in memory, and its published and newer warnings.
    fn drawn(
        &self,
        writer: &mut store::Writer,
        branch: u64,
        ordinal: usize,
        index: usize,
        interval: &BattleWindow,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Drawn<'_>>> {
        if self
            .paged_before
            .is_none_or(|before| interval.first_hour >= before)
        {
            return Ok(None);
        }
        let (first, last) = (interval.first_hour, interval.last_hour);
        // Its runs reach from its start to its last run, whatever it paged out of them.
        let falls_in = |episode: &Episode| {
            episode.start.hour <= last
                && episode
                    .runs
                    .last()
                    .is_some_and(|run| run.last_hour >= first)
                || episode
                    .end
                    .as_ref()
                    .is_some_and(|(hour, _)| (first..=last).contains(hour))
        };
        // A block pages battles that ended before its interval, so none earlier falls in this one.
        let mut paged = BTreeMap::new();
        for page in index + 1..ordinal {
            check_cancel(cancelled)?;
            let records: Vec<Paged> =
                read_page(writer, &format!("land-battle-paged-{branch}"), page)?;
            for record in records {
                // A battle paged again supersedes its earlier pages.
                if falls_in(&record.episode) {
                    paged.insert(record.at, record.episode);
                } else {
                    paged.remove(&record.at);
                }
            }
        }
        let mut episodes: BTreeMap<_, _> = paged
            .into_iter()
            .filter(|(at, _)| !self.episodes.contains_key(at))
            .map(|(at, episode)| (at, Cow::Owned(episode)))
            .collect();
        episodes.extend(
            self.episodes
                .iter()
                .filter(|(_, episode)| falls_in(episode))
                .map(|(at, episode)| (*at, Cow::Borrowed(episode))),
        );
        for episode in episodes.values_mut() {
            check_cancel(cancelled)?;
            if episode.history.is_some() {
                *episode = Cow::Owned(
                    slice(writer, branch, episode, (first, last), cancelled)?.into_owned(),
                );
            }
        }
        let published: Census = read_page(writer, &format!("land-battle-ranking-{branch}"), index)?;
        let mut warnings = published.warnings;
        warnings.merge(self.counts_in(&self.warnings, interval.first_hour, interval.last_hour));
        // A span still held may have grown since; one paged out had ended.
        let mut observed: BTreeMap<_, _> = published.observed.into_iter().collect();
        observed.extend(self.observed_spans());
        Ok(Some(Drawn {
            episodes,
            warnings,
            observed: observed.into_iter().collect(),
        }))
    }
    /// Decides pending finalizers in observation order, as memory would have: each ends
    /// its combat's newest paged battle or keeps its own. Returns the ordinals left unused.
    fn resolve(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        windows: &[BattleWindow],
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<BTreeSet<usize>> {
        let mut unused = BTreeSet::new();
        for Pending {
            combat,
            from_hour,
            at,
            own,
        } in std::mem::take(&mut self.pending)
        {
            // The combat's newest battle was its latest when paged; a battle
            // paged again supersedes its earlier pages.
            let mut candidate: Option<(usize, Episode)> = None;
            for page in 1..ordinal {
                check_cancel(cancelled)?;
                if self.interval_first(windows, page) <= from_hour {
                    continue;
                }
                let paged: Vec<Paged> =
                    read_page(writer, &format!("land-battle-paged-{branch}"), page)?;
                for paged in paged {
                    let newest = candidate.as_ref().map(|(at, _)| *at);
                    if newest == Some(paged.at)
                        || paged.combat == Some(combat) && newest.is_none_or(|at| paged.at > at)
                    {
                        candidate = Some((paged.at, paged.episode));
                    }
                }
            }
            let result = own.result.as_ref().expect("ended by its finalizer");
            let ended = candidate.filter(|(newest, episode)| {
                self.episodes.get(newest).unwrap_or(episode).ends(
                    own.province,
                    result.hour,
                    from_hour,
                )
            });
            match ended {
                Some((newest, episode)) => {
                    let mut episode = self.episodes.remove(&newest).unwrap_or(episode);
                    episode.result = own.result;
                    episode.end = own.end;
                    episode.losses.extend(own.losses);
                    self.episodes.insert(newest, episode);
                    self.latest.entry(combat).or_insert(newest);
                    self.dirty.insert(newest);
                    unused.insert(at);
                }
                None => {
                    self.started(own.start.hour);
                    self.episodes.insert(at, own);
                    self.dirty.insert(at);
                }
            }
        }
        Ok(unused)
    }
    /// Pages out every battle that ended before window `ordinal`'s ranking
    /// interval, and the warnings dated before it.
    fn page_out(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        windows: &[BattleWindow],
        ordinal: usize,
    ) -> io::Result<()> {
        let mut paged = Vec::new();
        #[cfg(test)]
        let keep = KEEP_ALL.get();
        #[cfg(not(test))]
        let keep = false;
        if ordinal > 0 && !keep {
            let before = self.interval_first(windows, ordinal);
            let ended: BTreeSet<usize> = self
                .episodes
                .iter()
                .filter(|(_, episode)| episode.end.as_ref().is_some_and(|(hour, _)| *hour < before))
                .map(|(at, _)| *at)
                .collect();
            let mut combats = HashMap::new();
            self.latest.retain(|&combat, at| {
                let stays = !ended.contains(at);
                if !stays {
                    combats.insert(*at, combat);
                }
                stays
            });
            for at in ended {
                paged.push(Paged {
                    at,
                    combat: combats.get(&at).copied(),
                    episode: self.episodes.remove(&at).expect("ended battle"),
                });
            }
            // An open battle keeps its runs from `before` and the loss read carried into it.
            // Pages are numbered as written: in battle order, not the open map's hash order.
            let mut open: Vec<usize> = self.open.values().copied().collect();
            open.sort_unstable();
            let mut spooled = Vec::new();
            for at in open {
                let episode = held(&mut self.episodes, at);
                let runs = episode
                    .runs
                    .partition_point(|run| run.last_hour < before)
                    .min(episode.runs.len().saturating_sub(1));
                let losses = episode
                    .losses
                    .partition_point(|read| read.hour <= before)
                    .saturating_sub(1);
                if runs == 0 && losses == 0 {
                    continue;
                }
                let runs: Vec<_> = episode.runs.drain(..runs).collect();
                episode.fold_peaks(&runs);
                let losses: Vec<_> = episode.losses.drain(..losses).collect();
                // The course query reads these as wired here, and the course page
                // goes on wiring reads where they leave off.
                let carried = episode.carried.get_or_insert_with(Default::default);
                let mut designs = Numbered::from_order(carried.designs.iter().copied());
                let wired = losses
                    .iter()
                    .map(|read| read.wire(&mut carried.previous, &mut designs))
                    .collect();
                carried.designs = designs.into_vec();
                let course = spool(
                    writer,
                    &format!("land-battle-course-history-{branch}"),
                    &Course {
                        runs,
                        losses: wired,
                        earlier: carried.course,
                    },
                    &mut spooled,
                )?;
                carried.course = Some(course);
                let earlier = Earlier {
                    losses,
                    course,
                    earlier: episode.history,
                    before,
                    peaks: episode.prior_peaks.clone(),
                };
                episode.history = Some(spool(
                    writer,
                    &format!("land-battle-history-{branch}"),
                    &earlier,
                    &mut spooled,
                )?);
            }
            self.warnings.retain_from(before);
            self.variants.keep_current();
            // The newest span to end before stays: the next window's records name it.
            let ended = self.observed.partition_point(|(_, last)| *last < before);
            self.observed.drain(..ended.saturating_sub(1));
            self.paged_before = Some(before);
        }
        series::write_reply(
            writer,
            &format!("land-battle-paged-{branch}"),
            ordinal as u64,
            &paged,
        )
    }
    /// Writes `episode`'s course page: the runs and reads it holds, wired on
    /// from those it paged out, which its history pages hold.
    fn write_course(
        &self,
        writer: &mut store::Writer,
        branch: u64,
        page: u64,
        episode: &Episode,
    ) -> io::Result<()> {
        let (mut previous, mut used) = match episode.carried.as_deref() {
            Some(carried) => (
                carried.previous.clone(),
                Numbered::from_order(carried.designs.iter().copied()),
            ),
            None => ([None, None], Numbered::default()),
        };
        let battle = episode.course(&mut previous, &mut used);
        series::write_reply(
            writer,
            &format!("land-battle-course-{branch}"),
            page,
            &CoursePage {
                history: episode.carried.as_ref().and_then(|carried| carried.course),
                reply: LandBattleCourseReply {
                    battle,
                    designs: self.reply_designs(used),
                    content_warnings: Vec::new(),
                },
            },
        )
    }
    pub fn publish_block(
        &mut self,
        writer: &mut store::Writer,
        replay: &Replay,
        branch: u64,
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.publish_block_in(writer, &windows(replay, branch), branch, ordinal, cancelled)
    }
    /// [`Self::publish_block`] with the branch's blocks, as the emit holds them.
    pub(super) fn publish_block_in(
        &mut self,
        writer: &mut store::Writer,
        windows: &[BattleWindow],
        branch: u64,
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let mut own = std::mem::take(&mut self.refusals);
        let published = dashboard_text::catalogue::own_refusals(&mut own, || {
            self.publish_own_block(writer, windows, branch, ordinal, cancelled)
        });
        self.refusals = own;
        published
    }
    fn publish_own_block(
        &mut self,
        writer: &mut store::Writer,
        windows: &[BattleWindow],
        branch: u64,
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        self.materialize()?;
        let window = &windows[ordinal];
        let boundary = if ordinal == 0 {
            i32::MIN
        } else {
            windows[ordinal - 1].last_hour.saturating_add(1)
        };
        self.windows.event(0, boundary, &(ordinal as u64))?;
        self.windows.through(window.last_hour);
        self.windows.flush(
            writer,
            &format!("land-battle-windows-{branch}"),
            0,
            cancelled,
        )?;

        let unused = self.resolve(writer, branch, windows, ordinal, cancelled)?;
        let mut changed = BTreeSet::from([ordinal]);
        let course = format!("land-battle-course-{branch}");
        for &at in self.dirty.union(&unused) {
            check_cancel(cancelled)?;
            let Some(episode) = self.episodes.get(&at) else {
                // The ordinal of a finalizer that ended a paged battle keeps the pages numbered.
                series::write_reply(writer, &course, at as u64, &())?;
                continue;
            };
            // A delayed finalizer can add terminal facts to an earlier block.
            if let Some(result) = &episode.result {
                for hour in [
                    result.hour,
                    episode.runs.last().expect("observed episode").last_hour,
                ] {
                    let index = windows.partition_point(|window| window.last_hour < hour);
                    if index <= ordinal && index < windows.len() {
                        changed.insert(index);
                    }
                }
            }
            self.write_course(writer, branch, at as u64, episode)?;
        }
        let mut ranking_changed = changed.clone();
        for at in &self.dirty {
            check_cancel(cancelled)?;
            let episode = &self.episodes[at];
            for hour in episode
                .end
                .as_ref()
                .map(|(hour, _)| *hour)
                .into_iter()
                .chain(episode.result.as_ref().map(|result| result.hour))
            {
                let index = windows.partition_point(|window| window.last_hour < hour);
                if index <= ordinal && index < windows.len() {
                    ranking_changed.insert(index);
                }
            }
        }
        for index in ranking_changed {
            check_cancel(cancelled)?;
            let interval = BattleWindow {
                first_hour: self.interval_first(windows, index),
                last_hour: windows[index].last_hour,
            };
            let drawn = self.drawn(writer, branch, ordinal, index, &interval, cancelled)?;
            series::write_reply(
                writer,
                &format!("land-battle-ranking-{branch}"),
                index as u64,
                &self.window_records(&interval, drawn.as_ref()),
            )?;
            if changed.contains(&index) {
                let window = &windows[index];
                let reply = self
                    .window_records(window, drawn.as_ref())
                    .window_reply(index, window);
                // One serialization is both the size the reply is held to and the page.
                let json = serde_json::to_vec(&reply)?;
                if json.len() > MAX_PRODUCT_REPLY_BYTES {
                    bounded(&reply)?;
                }
                series::write_compressed(
                    writer,
                    &format!("land-battles-{branch}"),
                    index as u64,
                    &series::compress(&json, 3)?,
                )?;
            }
        }
        self.dirty.clear();
        self.reads
            .flush(writer, &format!("land-battle-reads-{branch}"), 0, cancelled)?;
        // Each block lists the battles whose course pages it first published.
        let keys: Vec<(&str, u64)> = (self.keyed..self.created)
            .filter_map(|at| {
                self.episodes
                    .get(&at)
                    .map(|episode| (episode.id.as_str(), at as u64))
            })
            .collect();
        series::write_reply(
            writer,
            &format!("land-battle-keys-{branch}"),
            ordinal as u64,
            &keys,
        )?;
        self.keyed = self.created;
        series::write_reply(
            writer,
            &format!("land-battle-availability-{branch}"),
            0,
            &self.available,
        )?;
        series::write_reply(
            writer,
            &format!("land-battle-warnings-{branch}"),
            0,
            &self.content_warnings(),
        )?;
        self.page_out(writer, branch, windows, ordinal)
    }
}

/// Committed log pages remain useful while later windows are pending.
pub fn prefix_query(
    source: &Source,
    product: &str,
    params: &Value,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Option<Value>> {
    let id = params["branchId"]
        .as_str()
        .ok_or_else(|| dashboard_core::invalid("Invalid battle branch"))?;
    let branch = branch_id(source, id)?;
    let prefix = dashboard_base::prepared::prefix::read_or_empty(
        source,
        context.content.as_ref(),
        product,
        branch,
        None,
        cancelled,
    )?;
    let windows = windows(source.replay_ref(), branch);
    let mut data = match product {
        "land-battles-manifest" => {
            let available = series::read_branch_reply::<bool>(
                store,
                key,
                &format!("land-battle-availability-{branch}"),
                0,
                branch,
            )?
            .unwrap_or(false);
            serde_json::to_value(LandBattlesManifestReply {
                windows,
                available,
                unavailable: None,
                content_warnings: Vec::new(),
            })?
        }
        "land-battles" => {
            let params: LandBattlesParams = serde_json::from_value(params.clone())?;
            LandBattles::validate(&params).map_err(dashboard_core::invalid)?;
            let window = windows
                .get(params.window)
                .ok_or_else(|| dashboard_core::invalid("Battle window not in this recording"))?;
            let reply = if window.last_hour <= prefix.coverage.last_hour {
                series::read_branch_reply::<LandBattlesReply>(
                    store,
                    key,
                    &format!("land-battles-{branch}"),
                    params.window as u64,
                    branch,
                )?
            } else {
                None
            };
            serde_json::to_value(reply.unwrap_or_else(|| LandBattlesReply {
                window: params.window,
                interval: window.clone(),
                rows: Vec::new(),
                observed: Vec::new(),
                reasons: Vec::new(),
                warnings: Vec::new(),
                content_warnings: Vec::new(),
            }))?
        }
        _ => return Ok(None),
    };
    // Later terminal reads can repair earlier battle rows.
    prefix.mark_revisable(source, branch, &mut data)?;
    data["warningsComplete"] = Value::Bool(false);
    Ok(Some(data))
}

/// `reply` written once, refused as the transport refuses a reply past its budget: what bounds a
/// reply its own product leaves unbounded.
fn transported(reply: &impl Serialize, cancelled: &AtomicBool) -> io::Result<Built> {
    Built::within(reply, cancelled, dashboard_core::transport::MAX_REPLY_BYTES, || {
        dashboard_core::invalid("Projected response exceeds the transport byte budget")
    })
}

pub fn query(
    source: &Source,
    product: &str,
    params: Value,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Built> {
    let id = params
        .get("branchId")
        .and_then(Value::as_str)
        .ok_or_else(|| dashboard_core::invalid("Invalid battle branch"))?;
    let branch = branch_id(source, id)?;
    match product {
        "land-battles-manifest" => {
            let params: LandBattlesManifestParams = serde_json::from_value(params)?;
            LandBattlesManifest::validate(&params).map_err(dashboard_core::invalid)?;
            let available: bool =
                series::read_reply(store, key, &format!("land-battle-availability-{branch}"), 0)?
                    .ok_or_else(|| {
                    dashboard_core::invalid("Prepared land battle availability is missing")
                })?;
            let windows = windows(source.replay_ref(), branch);
            Built::within(
                &LandBattlesManifestReply {
                    windows,
                    available,
                    unavailable: (!available).then(|| "Not in this recording".into()),
                    content_warnings: Vec::new(),
                },
                cancelled,
                MAX_PRODUCT_REPLY_BYTES,
                too_long,
            )
        }
        "land-battle-course" => {
            let warnings: Vec<String> =
                series::read_reply(store, key, &format!("land-battle-warnings-{branch}"), 0)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared land battle diagnostics are missing")
                    })?;
            let params: LandBattleCourseParams = serde_json::from_value(params)?;
            LandBattleCourses::validate(&params).map_err(dashboard_core::invalid)?;
            let section = format!("land-battle-keys-{branch}");
            let count = store.range_count(key, &section)?.unwrap_or(0);
            if count == 0 {
                return Err(dashboard_core::invalid(
                    "Prepared battle catalogue is missing",
                ));
            }
            // Finalizers of one combat can repeat an id; the last published answers.
            let mut page = None;
            for index in (0..count).rev() {
                check_cancel(cancelled)?;
                let keys: Vec<(String, u64)> = series::read_reply(store, key, &section, index)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared battle catalogue is missing")
                    })?;
                page = keys
                    .into_iter()
                    .rev()
                    .find_map(|(id, page)| (id == params.id).then_some(page));
                if page.is_some() {
                    break;
                }
            }
            let page =
                page.ok_or_else(|| dashboard_core::invalid("Battle not in this recording branch"))?;
            let course: CoursePage =
                series::read_reply(store, key, &format!("land-battle-course-{branch}"), page)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared land battle course is missing")
                    })?;
            let reply = stored_course(store, key, branch, course, warnings, cancelled)?;
            transported(&reply, cancelled)
        }
        "land-battles" => {
            let warnings: Vec<String> =
                series::read_reply(store, key, &format!("land-battle-warnings-{branch}"), 0)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared land battle diagnostics are missing")
                    })?;
            let params: LandBattlesParams = serde_json::from_value(params)?;
            LandBattles::validate(&params).map_err(dashboard_core::invalid)?;
            let mut reply: LandBattlesReply = series::read_reply(
                store,
                key,
                &format!("land-battles-{branch}"),
                params.window as u64,
            )?
            .ok_or_else(|| dashboard_core::invalid("Battle window not in this recording"))?;
            reply.content_warnings = warnings;
            Built::within(&reply, cancelled, MAX_PRODUCT_REPLY_BYTES, too_long)
        }
        "land-battles-ranking" => {
            let warnings: Vec<String> =
                series::read_reply(store, key, &format!("land-battle-warnings-{branch}"), 0)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared land battle diagnostics are missing")
                    })?;
            let params: LandBattlesRankingParams = serde_json::from_value(params)?;
            LandBattlesRanking::validate(&params).map_err(dashboard_core::invalid)?;
            let mut census = Census::default();
            let mut episodes = HashMap::new();
            let mut index = series::Reader::new(
                store,
                key,
                format!("land-battle-windows-{branch}"),
                cancelled,
            )?;
            let first = index
                .latest_event(0, params.first_hour)?
                .map(|(_, value)| serde_json::from_slice::<u64>(&value))
                .transpose()?
                .unwrap_or(0);
            let last = index
                .latest_event(0, params.last_hour)?
                .map(|(_, value)| serde_json::from_slice::<u64>(&value))
                .transpose()?
                .unwrap_or(0);
            for index in first..=last {
                check_cancel(cancelled)?;
                let page: Census = series::read_reply(
                    store,
                    key,
                    &format!("land-battle-ranking-{branch}"),
                    index,
                )?
                .ok_or_else(|| dashboard_core::invalid("Prepared land battle window is missing"))?;
                for episode in page.episodes.into_values() {
                    episodes.insert(episode.id.clone(), episode);
                }
                census.warnings.merge(page.warnings);
            }
            census.episodes = episodes.into_values().enumerate().collect();
            let mut reads =
                series::Reader::new(store, key, format!("land-battle-reads-{branch}"), cancelled)?;
            census.observed = reads
                .latest_event(0, params.last_hour)?
                .map(|(hour, _)| (hour, hour))
                .into_iter()
                .collect();
            census.content_warnings = warnings;
            transported(&census.ranked(&params, cancelled)?, cancelled)
        }
        _ => unreachable!(),
    }
}
