//! The map's script-driven readers at the playback head: the raid layer's offer cycle and music
//! chances, evaluated through the content's trigger evaluator over rows at the head
//! ([`crate::playhead_host::rows`]) on a worker of their own, never on the frame thread.
//!
//! One reader of rows per map view; its tables are those the scripts read, learned by discovery.
//! Rows come at the step that changes a table the scripts read while playing, and at once when
//! paused, after a seek or when a table arrives. Each new frame evaluates the viewing country's music chances and one
//! batch of the raid offer cycle; a frame published while a batch runs is replaced by the next
//! one (counted as skipped), never queued. The cycle is the layer's own state: it carries from
//! frame to frame and starts again at a playhead reset (a seek, a piece's first hour), a new
//! viewing country, new content or a step back. Paused, it evaluates one batch per frame the
//! window draws until every target is current, as the game's player cache does.

use crate::{
    document::Content,
    playhead_host::{
        Entry,
        rows::{Cadence, Need, Newest, Recording, RowReader, Rows},
    },
};
use dashboard_text::triggers::World;
use replay_data::{
    Hour,
    rows::{TableId, tables},
};
use replay_frame::frame::{DecodedFrame, discovering_fields};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Condvar, Mutex, MutexGuard, Weak},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use viewer_foundation::map::{perspective, tables::core};
use viewer_foundation::trigger_content::TriggerContent;
use viewer_raids::map::raids::{HourLayer, IconCache, RaidAssets, RaidIcons, RaidReport, cycle::Cycle, paths::HourPaths};

/// How often the worker looks again when nothing wakes it: a frame at 60 Hz.
const TICK: Duration = Duration::from_micros(16_667);

/// Rows the scripts read four times a second while playing.
const CADENCE: Cadence = Cadence::Ready;

/// The core of the trigger evaluator, declared by its keys before any script runs: the columns
/// come from the reads, as every other table's do.
const CORE: [TableId; 4] = [TableId::Countries, TableId::CountryLinks, TableId::Provinces, TableId::States];

/// What was read undeclared: each table with the fields it was read for.
type Missed = Vec<(TableId, u64)>;

/// `needs` with `missed` joined in: a table once, with the union of the fields read of it.
fn joined(needs: &mut Vec<Need>, missed: impl IntoIterator<Item = (TableId, u64)>) {
    for (table, fields) in missed {
        match needs.iter_mut().find(|need| need.table == table) {
            Some(need) => need.columns |= fields,
            None => needs.push(Need::columns(table, fields)),
        }
    }
    needs.sort_unstable_by_key(|need| need.table.index());
}

/// What the scripts evaluate with: the content's raid assets, its trigger facts for music, its
/// music and the executable tables its work reads.
#[derive(Clone, Copy)]
pub struct Sources<'a> {
    pub raids: &'a Arc<RaidAssets>,
    pub triggers: &'a Arc<TriggerContent>,
    pub audio: Option<&'a Arc<viewer_audio::Content>>,
    pub tables: Option<engine_executable::executable_tables::Tables>,
}

impl<'a> Sources<'a> {
    pub fn of(content: &'a Content) -> Self {
        Self { raids: &content.icons.raids, triggers: &content.products.triggers, audio: content.products.audio.as_ref(), tables: content.tables }
    }
}

#[derive(Clone)]
struct Held {
    raids: Arc<RaidAssets>,
    triggers: Arc<TriggerContent>,
    audio: Option<Arc<viewer_audio::Content>>,
    tables: Option<engine_executable::executable_tables::Tables>,
}

impl Held {
    fn is(&self, sources: &Sources<'_>) -> bool {
        Arc::ptr_eq(&self.raids, sources.raids) && Arc::ptr_eq(&self.triggers, sources.triggers) && self.audio.as_ref().map(Arc::as_ptr) == sources.audio.map(Arc::as_ptr)
    }

    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.raids, &other.raids) && Arc::ptr_eq(&self.triggers, &other.triggers) && self.audio.as_ref().map(Arc::as_ptr) == other.audio.as_ref().map(Arc::as_ptr)
    }
}

/// What the window shows this frame, as the scripts need it.
pub struct Ask<'a> {
    pub entry: &'a Arc<Entry>,
    pub recording: Recording,
    pub sources: Sources<'a>,
    pub perspective: &'a perspective::Choice,
    pub playing: bool,
    /// Raid icons are drawn.
    pub raids: bool,
    /// This map plays music.
    pub music: bool,
}

#[derive(Clone)]
struct Wanted {
    entry: Arc<Entry>,
    recording: Recording,
    content: Held,
    perspective: perspective::Choice,
    playing: bool,
    raids: bool,
    music: bool,
}

impl Wanted {
    fn is(&self, ask: &Ask<'_>) -> bool {
        Arc::ptr_eq(&self.entry, ask.entry)
            && self.recording == ask.recording
            && self.content.is(&ask.sources)
            && self.perspective == *ask.perspective
            && (self.playing, self.raids, self.music) == (ask.playing, ask.raids, ask.music)
    }

    /// What decides an evaluation's result, beside the frame.
    fn evaluates_like(&self, other: &Self) -> bool {
        self.content.same(&other.content) && self.perspective == other.perspective && (self.raids, self.music) == (other.raids, other.music)
    }
}

/// What the worker measured, while timing is on.
#[derive(Clone, Debug, Default)]
pub struct Timings {
    /// Per frame evaluated: its hour, the raid batch's and the music's nanoseconds.
    pub frames: Vec<(Hour, Option<u64>, Option<u64>)>,
    /// Each further batch of a paused cycle, nanoseconds.
    pub paused: Vec<u64>,
    /// Frames replaced by a newer one before they were evaluated.
    pub skipped: u64,
    /// Evaluations dropped because they read a table not declared yet, which was then declared.
    pub learned: u64,
    /// Times the cycle started again.
    pub restarts: u64,
    /// The thread the scripts ran on.
    pub ran_on: Option<String>,
}

#[derive(Default)]
struct State {
    wanted: Option<Wanted>,
    /// Paused frames the window drew: each lets a paused cycle evaluate one batch.
    drawn: u64,
    stop: bool,
    raids: Option<Arc<RaidIcons>>,
    /// The raid arrows' limitation notes of the rows the newest raid icons were built from, with
    /// the canonical viewing country they are for.
    raid_notes: Option<(u32, Arc<Vec<String>>)>,
    /// The music chances and the hour of the rows they were evaluated over.
    music: Option<(Hour, Arc<viewer_audio::Chances>)>,
    /// The tables the reader declares, each with the fields its reads named.
    needs: Vec<Need>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    timings: Option<Timings>,
    /// Bumped by every publish, evaluation, table learned and close.
    rounds: u64,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One map view's script-driven readers at the head; a worker started on the first frame that asks.
#[derive(Default)]
pub struct Scripts {
    shared: Shared,
    worker: Option<JoinHandle<()>>,
    asked: bool,
}

impl Scripts {
    /// Readers that declare `needs` from the start (the keys of a table, or the fields an earlier
    /// run's reads recorded), so their lanes have joined when the reads come: for the proof tool,
    /// whose head holds still at every comparison, where a table learned would arrive only later.
    pub fn primed(needs: &[Need]) -> Self {
        let scripts = Self::default();
        lock(&scripts.shared).needs = needs.to_vec();
        scripts
    }

    /// What the window shows this frame; a paused frame lets the cycle evaluate one batch. `wake`
    /// repaints the window when new results arrive.
    pub fn show(&mut self, ask: Ask<'_>, wake: impl Fn() + Send + Sync + 'static) {
        self.asked = true;
        if self.worker.is_none() {
            let shared = Arc::clone(&self.shared);
            self.worker = std::thread::Builder::new()
                .name("scripts at the head".into())
                .spawn(move || {
                    viewer_foundation::worker_priority::background_priority();
                    Worker::new(shared).run()
                })
                .map_err(|error| eprintln!("The map's scripts at the head are refused: {error}"))
                .ok();
        }
        let mut state = lock(&self.shared);
        let changed = !state.wanted.as_ref().is_some_and(|wanted| wanted.is(&ask));
        if changed {
            state.wanted = Some(Wanted {
                entry: Arc::clone(ask.entry),
                recording: ask.recording,
                content: Held { raids: Arc::clone(ask.sources.raids), triggers: Arc::clone(ask.sources.triggers), audio: ask.sources.audio.cloned(), tables: ask.sources.tables },
                perspective: ask.perspective.clone(),
                playing: ask.playing,
                raids: ask.raids,
                music: ask.music,
            });
        }
        if !ask.playing {
            state.drawn += 1;
        }
        if state.wake.is_none() {
            state.wake = Some(Arc::new(wake));
        }
        drop(state);
        if changed || !ask.playing {
            self.shared.1.notify_all();
        }
    }

    /// Closes the readers unless a frame asked since the last call: their tables are retired and
    /// nothing is held.
    pub fn end_frame(&mut self) {
        if std::mem::take(&mut self.asked) {
            return;
        }
        let mut state = lock(&self.shared);
        if state.wanted.take().is_some() {
            state.raids = None;
            state.raid_notes = None;
            state.music = None;
            drop(state);
            self.shared.1.notify_all();
        }
    }

    /// The newest raid icons built at the head for `branch` and canonical viewing country `ours`,
    /// at whatever hour the head was; none while loading or for another country.
    pub fn raids(&self, branch: replay_data::BranchId, ours: u32) -> Option<Arc<RaidIcons>> {
        lock(&self.shared).raids.clone().filter(|icons| match icons.built_for {
            Some((built, _, country)) => (built, country) == (branch, ours),
            None => icons.report.worker_failed,
        })
    }

    /// The raid arrows' limitation notes in today's words, of the rows the newest raid icons were
    /// built from, for canonical viewing country `ours`; none while loading, for another country,
    /// or while raid icons are not drawn.
    pub fn raid_notes(&self, ours: u32) -> Option<Arc<Vec<String>>> {
        lock(&self.shared).raid_notes.as_ref().filter(|(country, _)| *country == ours).map(|(_, notes)| Arc::clone(notes))
    }

    /// The newest music chances at the head; none while loading or when music is not asked.
    pub fn music(&self) -> Option<Arc<viewer_audio::Chances>> {
        lock(&self.shared).music.as_ref().map(|(_, music)| Arc::clone(music))
    }

    /// The newest music chances with the hour they were evaluated at.
    pub fn music_at(&self) -> Option<(Hour, Arc<viewer_audio::Chances>)> {
        lock(&self.shared).music.clone()
    }

    /// The hour of the rows the newest raid icons were built from.
    pub fn newest_hour(&self) -> Option<Hour> {
        lock(&self.shared).raids.as_ref().and_then(|icons| icons.built_for).map(|(_, hour, _)| hour)
    }

    /// Times the worker's evaluations from now on ([`Self::timings`]).
    pub fn time(&self) {
        lock(&self.shared).timings.get_or_insert_with(Timings::default);
    }

    pub fn timings(&self) -> Option<Timings> {
        lock(&self.shared).timings.clone()
    }

    /// The tables the reader declares now.
    pub fn tables(&self) -> Vec<TableId> {
        lock(&self.shared).needs.iter().map(|need| need.table).collect()
    }

    /// The tables the reader declares now, each with the fields its reads named.
    pub fn needs(&self) -> Vec<Need> {
        lock(&self.shared).needs.clone()
    }

    /// The worker's rounds so far ([`Self::wait_round`]).
    #[cfg(test)]
    pub fn rounds(&self) -> u64 {
        lock(&self.shared).rounds
    }

    /// Waits up to `limit` for a round after the `after`th: a publish, an evaluation, a table
    /// learned or a close; the rounds then.
    #[cfg(test)]
    pub fn wait_round(&self, after: u64, limit: Duration) -> u64 {
        let until = Instant::now() + limit;
        let mut state = lock(&self.shared);
        while state.rounds <= after {
            let now = Instant::now();
            if now >= until {
                break;
            }
            state = self.shared.1.wait_timeout(state, until - now).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
        state.rounds
    }
}

impl Drop for Scripts {
    fn drop(&mut self) {
        lock(&self.shared).stop = true;
        self.shared.1.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The offer cycle since its last start, and what started it.
struct Chain {
    resets: u64,
    content: Weak<RaidAssets>,
    country: perspective::Country,
    /// The hour of the frame it last evaluated.
    hour: Hour,
    cycle: Cycle,
    icons: IconCache,
}

struct Worker {
    shared: Shared,
    reader: Option<(Weak<Entry>, RowReader)>,
    needs: Vec<Need>,
    /// The publish count of the frame last evaluated, and what it was evaluated for.
    seen: u64,
    evaluated: Option<Wanted>,
    chain: Option<Chain>,
    /// A reset whose batch panicked: nothing more is evaluated until the next.
    failed: Option<u64>,
    memo: map_products::music_inputs::Memo,
    kept: Option<map_products::music_inputs::Kept>,
    /// The paused frames drawn when a paused cycle last went on: once per drawn frame.
    resumed: u64,
}

/// `read`, the tables it read undeclared, with the fields it read, added to `missed`.
fn learning<T>(missed: &mut Missed, read: impl FnOnce() -> T) -> T {
    let (value, tables) = discovering_fields(read);
    missed.extend(tables);
    value
}

/// The canonical slot `choice` views the map as at `frame`'s hour.
/// The raid arrows' limitation notes of canonical viewing country `ours` at `frame`, in today's
/// words: stated from a wholly recorded raid census only.
fn raid_notes(frame: &DecodedFrame, ours: u32) -> Vec<String> {
    let mut out = viewer_foundation::map::tables::Limitations::default();
    if let Ok(raids) = frame.complete_fields::<tables::Raids>(replay_data::rows::mask::<replay_data::rows::RaidRow>(&["parent_slot", "phase"])) {
        let own: Vec<_> = raids.iter().filter(|raid| raid.parent_slot == ours && raid.phase != 5).collect();
        out.note_raid_arrows(own.len(), own.iter().filter(|raid| raid.phase == 4).count());
    }
    out.into_lines()
}

fn ours(frame: &DecodedFrame, choice: &perspective::Choice) -> io::Result<u32> {
    let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?.iter().map(|link| (link.id, link.canonical_slot)).collect();
    let tags: BTreeMap<u32, String> = core::<tables::Countries>(frame)?.iter().map(|country| (country.canonical_slot, String::from_utf8_lossy(&country.tag).into_owned())).collect();
    perspective::viewer(frame, &links, &tags, &choice.country).map(|(slot, _)| slot)
}

impl Worker {
    fn new(shared: Shared) -> Self {
        let mut needs = lock(&shared).needs.clone();
        joined(&mut needs, CORE.iter().map(|&table| (table, 0)));
        // The scripts compare dates: their rows come at every step.
        joined(&mut needs, [(TableId::Clock, u64::MAX)]);
        Self { shared, reader: None, needs, seen: 0, evaluated: None, chain: None, failed: None, memo: map_products::music_inputs::Memo::by_identity(), kept: None, resumed: 0 }
    }

    fn run(mut self) {
        loop {
            let (wanted, drawn) = {
                let state = lock(&self.shared);
                if state.stop {
                    return;
                }
                (state.wanted.clone().filter(|wanted| wanted.raids || wanted.music), state.drawn)
            };
            let Some(wanted) = wanted else {
                self.close();
                let mut state = lock(&self.shared);
                while !state.stop && !state.wanted.as_ref().is_some_and(|wanted| wanted.raids || wanted.music) {
                    state = self.shared.1.wait(state).unwrap_or_else(std::sync::PoisonError::into_inner);
                }
                continue;
            };
            if !self.open(&wanted) {
                let state = lock(&self.shared);
                drop(self.shared.1.wait_timeout(state, TICK));
                continue;
            }
            let newest = self.reader.as_ref().and_then(|(_, reader)| reader.newest());
            match newest {
                Some(newest) if newest.published != self.seen || !self.evaluated.as_ref().is_some_and(|before| before.evaluates_like(&wanted)) => {
                    let skipped = newest.published.saturating_sub(self.seen + 1);
                    self.seen = newest.published;
                    self.evaluated = Some(wanted.clone());
                    if let Some(timings) = &mut lock(&self.shared).timings {
                        timings.skipped += skipped;
                    }
                    self.evaluate(&wanted, &newest, true);
                }
                // Paused on a frame evaluated while playing: its cycle goes on until complete.
                Some(newest) if !wanted.playing && wanted.raids && self.resumed != drawn && self.failed != Some(newest.resets) && self.chain.as_ref().is_some_and(|chain| !chain.cycle.complete() && chain.hour == newest.frame.hour) => {
                    self.resumed = drawn;
                    self.evaluate(&wanted, &newest, false);
                }
                _ => {
                    if let Some((_, reader)) = &self.reader {
                        reader.ready();
                        let _ = reader.wait_newer(self.seen, TICK);
                    }
                }
            }
        }
    }

    /// Whether the reader on `wanted`'s playhead is open, opening it when it is not.
    fn open(&mut self, wanted: &Wanted) -> bool {
        if !self.reader.as_ref().is_some_and(|(held, _)| std::ptr::eq(held.as_ptr(), Arc::as_ptr(&wanted.entry))) {
            self.close();
            match Rows::of(&wanted.entry, wanted.recording) {
                Ok(rows) => self.reader = Some((Arc::downgrade(&wanted.entry), rows.reader_of(&self.needs, CADENCE))),
                Err(error) => {
                    eprintln!("The map's scripts at the head are refused: {error}");
                    return false;
                }
            }
            lock(&self.shared).needs.clone_from(&self.needs);
        }
        true
    }

    /// Lets go of the reader and the cycle; the tables learned are kept for the next open.
    fn close(&mut self) {
        self.reader = None;
        self.chain = None;
        self.kept = None;
        self.evaluated = None;
        self.seen = 0;
        let mut state = lock(&self.shared);
        state.raids = None;
        state.raid_notes = None;
        state.music = None;
        state.rounds += 1;
        drop(state);
        self.shared.1.notify_all();
    }

    /// Declares `missed` too; its rows arrive off the frame and the next frame is evaluated.
    fn learn(&mut self, missed: Missed) {
        joined(&mut self.needs, missed);
        if let Some((_, reader)) = &self.reader {
            reader.declare_needs(&self.needs);
        }
        let mut state = lock(&self.shared);
        state.needs.clone_from(&self.needs);
        state.rounds += 1;
        if let Some(timings) = &mut state.timings {
            timings.learned += 1;
        }
        drop(state);
        self.shared.1.notify_all();
    }

    fn publish(&self, raids: Option<Option<Arc<RaidIcons>>>, music: Option<Option<(Hour, Arc<viewer_audio::Chances>)>>) {
        self.publish_with(raids, music, None);
    }

    /// [`Self::publish`], with the raid arrows' notes of the icons published (none when no icons
    /// of a cycle are).
    fn publish_with(&self, raids: Option<Option<Arc<RaidIcons>>>, music: Option<Option<(Hour, Arc<viewer_audio::Chances>)>>, notes: Option<(u32, Arc<Vec<String>>)>) {
        let mut state = lock(&self.shared);
        if let Some(raids) = raids {
            state.raids = raids;
            state.raid_notes = notes;
        }
        if let Some(music) = music {
            state.music = music;
        }
        state.rounds += 1;
        let wake = state.wake.clone();
        drop(state);
        self.shared.1.notify_all();
        if let Some(wake) = wake {
            wake();
        }
    }

    /// Music (unless `again`: the same frame as before) and the raid cycle at `newest`'s frame.
    fn evaluate(&mut self, wanted: &Wanted, newest: &Newest, again: bool) {
        let frame: &DecodedFrame = &newest.frame;
        let mut missed = Vec::new();
        // A round that reads a table not declared yet goes on to the end, so it learns every table
        // it reaches at once, and publishes nothing.
        let ours = learning(&mut missed, || ours(frame, &wanted.perspective)).map_err(|error| error.to_string());
        let music = if again {
            let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine_executable::executable_tables::scope(wanted.content.tables, || self.music(wanted, frame, ours.clone(), &mut missed))));
            evaluated.unwrap_or_else(|_| {
                self.kept = None;
                self.publish(None, Some(None));
                None
            })
        } else {
            None
        };
        let raids = if wanted.raids {
            let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine_executable::executable_tables::scope(wanted.content.tables, || self.raids(wanted, newest, ours, &mut missed))));
            evaluated.unwrap_or_else(|_| {
                self.chain = None;
                self.failed = Some(newest.resets);
                self.publish(Some(Some(Arc::new(RaidIcons { report: RaidReport { worker_failed: true, ..Default::default() }, ..Default::default() }))), None);
                None
            })
        } else {
            self.chain = None;
            self.publish(Some(None), None);
            None
        };
        if !missed.is_empty() {
            self.learn(missed);
        }
        let mut state = lock(&self.shared);
        state.rounds += 1;
        if let Some(timings) = &mut state.timings {
            timings.frames.push((frame.hour, raids, music));
            timings.ran_on = std::thread::current().name().map(str::to_owned);
        }
        drop(state);
        self.shared.1.notify_all();
    }

    /// The music chances of the viewing country at `frame`; the nanoseconds they took.
    fn music(&mut self, wanted: &Wanted, frame: &DecodedFrame, ours: Result<u32, String>, missed: &mut Missed) -> Option<u64> {
        let audio = wanted.content.audio.as_ref().filter(|audio| wanted.music && !audio.music.songs.is_empty());
        let (Some(audio), Ok(ours)) = (audio, ours) else {
            if missed.is_empty() {
                self.kept = None;
                self.publish(None, Some(None));
            }
            return None;
        };
        let started = Instant::now();
        let (chances, kept) = learning(missed, || {
            let world = World::read(frame, wanted.content.triggers.facts());
            self.memo.chances_kept(frame, audio, ours, &world)
        });
        if missed.is_empty() {
            self.publish(None, Some(Some((kept.hour(), Arc::new(chances)))));
            self.kept = Some(kept);
        }
        Some(started.elapsed().as_nanos() as u64)
    }

    /// One batch of the offer cycle at `newest`'s frame, from the cycle after the frame before;
    /// paused, a batch more per frame the window draws until every target is current. The first
    /// batch's nanoseconds.
    fn raids(&mut self, wanted: &Wanted, newest: &Newest, ours: Result<u32, String>, missed: &mut Missed) -> Option<u64> {
        let frame: &DecodedFrame = &newest.frame;
        let assets = &wanted.content.raids;
        let ours = match ours {
            Ok(ours) => ours,
            Err(error) => {
                if missed.is_empty() {
                    self.publish(Some(Some(Arc::new(RaidIcons::unavailable(frame, 0, error)))), None);
                }
                return None;
            }
        };
        let restart = self.chain.as_ref().is_none_or(|chain| {
            chain.resets != newest.resets || !std::ptr::eq(chain.content.as_ptr(), Arc::as_ptr(&wanted.content.raids)) || chain.country != wanted.perspective.country || frame.hour < chain.hour
        });
        if restart {
            self.chain = Some(Chain { resets: newest.resets, content: Arc::downgrade(&wanted.content.raids), country: wanted.perspective.country.clone(), hour: frame.hour, cycle: Cycle::new(frame.branch, ours), icons: IconCache::default() });
            if let Some(timings) = &mut lock(&self.shared).timings {
                timings.restarts += 1;
            }
        }
        if self.failed == Some(newest.resets) {
            return None;
        }
        let started = Instant::now();
        let world = match learning(missed, || World::new(frame, assets.trigger_content.facts())) {
            Ok(world) => world,
            Err(reason) => {
                if missed.is_empty() {
                    self.publish(Some(Some(Arc::new(RaidIcons::unavailable(frame, ours, reason.to_string())))), None);
                }
                return None;
            }
        };
        let paths = learning(missed, || HourPaths::new(assets, frame));
        let layer = learning(missed, || HourLayer::new(assets, &world, ours, &paths));
        let chain = self.chain.as_mut().expect("started above");
        let mut cycle = match chain.cycle.clone() {
            cycle if cycle.ours() == ours && cycle.branch() == frame.branch => cycle,
            _ => Cycle::new(frame.branch, ours),
        };
        let icons = learning(missed, || {
            if let Err(reason) = cycle.begin(assets, &layer.sweep) {
                cycle = Cycle::restarted(frame.branch, ours, reason);
                // A cycle with no row always begins.
                let _ = cycle.begin(assets, &layer.sweep);
            }
            cycle.step(&layer.sweep);
            layer.icons(&cycle, &mut chain.icons)
        });
        let notes = learning(missed, || raid_notes(frame, ours));
        if !missed.is_empty() {
            chain.icons = IconCache::default();
            return None;
        }
        let first = started.elapsed().as_nanos() as u64;
        chain.cycle = cycle;
        chain.hour = frame.hour;
        let notes = Arc::new(notes);
        self.publish_with(Some(Some(Arc::new(icons))), None, Some((ours, Arc::clone(&notes))));
        let mut drawn = lock(&self.shared).drawn;
        while !wanted.playing {
            if self.chain.as_ref().is_none_or(|chain| chain.cycle.complete()) || !self.still(wanted, &mut drawn) {
                break;
            }
            let chain = self.chain.as_mut().expect("checked above");
            let began = Instant::now();
            let mut cycle = chain.cycle.clone();
            let icons = learning(missed, || {
                cycle.step(&layer.sweep);
                layer.icons(&cycle, &mut chain.icons)
            });
            if !missed.is_empty() {
                chain.icons = IconCache::default();
                break;
            }
            chain.cycle = cycle;
            self.publish_with(Some(Some(Arc::new(icons))), None, Some((ours, Arc::clone(&notes))));
            if let Some(timings) = &mut lock(&self.shared).timings {
                timings.paused.push(began.elapsed().as_nanos() as u64);
            }
        }
        Some(first)
    }

    /// Waits for the window to draw another paused frame; false when the view changed, a newer
    /// frame was published or the worker stops, so the paused cycle ends here.
    fn still(&self, wanted: &Wanted, drawn: &mut u64) -> bool {
        let mut state = lock(&self.shared);
        loop {
            let current = !state.stop && state.wanted.as_ref().is_some_and(|now| now.evaluates_like(wanted) && !now.playing && Arc::ptr_eq(&now.entry, &wanted.entry));
            let newer = self.reader.as_ref().and_then(|(_, reader)| reader.newest()).is_some_and(|newest| newest.published != self.seen);
            if !current || newer {
                return false;
            }
            if state.drawn != *drawn {
                *drawn = state.drawn;
                return true;
            }
            state = self.shared.1.wait_timeout(state, TICK).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }
}

pub mod proof;
#[cfg(test)]
mod tests;
