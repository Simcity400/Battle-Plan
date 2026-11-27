use crate::{Answer, Follow, Identity, Message, Query, Rows, SeriesMode, Sink, State, StepStats, answer_at, decode};
use dashboard_declare::{Carry, Column, Key, Measure, Parts, Series};
use dashboard_query::{
    coverage::CoverageScope,
    identity::{Holder, HolderRun, Holders},
    timeline::{Join, Stretch, Timeline as Stretches},
};
use playhead::{
    LogReads, Playhead, Settings, Timeline, Wanted,
    lane::Lane,
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::{Store, TableChanges},
    unit::{BLOCK, BlockPage, Encoder, PartSink, Settings as Writing, reader::Extents},
};
use replay_data::{Hour, rows::TableId};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Condvar, Mutex, mpsc},
};

const TABLE: usize = 2;
const KINDS: [Kind; 5] = [Kind::I32, Kind::Bytes, Kind::OptionalI64, Kind::I64, Kind::I64];
const HOURS: u32 = 2 * BLOCK;
const RING: usize = 3;

#[derive(Default)]
struct Memory {
    bytes: Vec<u8>,
    pages: Vec<BlockPage>,
}

impl PartSink for Memory {
    fn frame(&mut self, stored: &[u8]) -> io::Result<u64> {
        let offset = self.bytes.len() as u64;
        self.bytes.extend_from_slice(stored);
        Ok(offset)
    }

    fn block(&mut self, page: BlockPage) -> io::Result<()> {
        self.pages.push(page);
        Ok(())
    }

    fn read_back(&mut self, offset: u64, stored: u32, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        out.extend_from_slice(&self.bytes[offset as usize..offset as usize + stored as usize]);
        Ok(())
    }
}

struct Recording {
    bytes: Vec<u8>,
    runs: Vec<Run>,
    kinds: Vec<Vec<Kind>>,
}

impl Pages for Recording {
    fn extents(&self, _run: usize) -> &dyn Extents {
        &self.bytes
    }

    fn runs(&self) -> &[Run] {
        &self.runs
    }

    fn kinds(&self, table: usize) -> Option<&[Kind]> {
        self.kinds.get(table).map(Vec::as_slice)
    }
}

fn record(hour: impl FnMut(u32, &mut TableChanges<'_>)) -> Arc<Recording> {
    record_with(|_| Status::NotRecorded, hour)
}

fn record_with(mut gap: impl FnMut(&mut Store) -> Status, mut hour: impl FnMut(u32, &mut TableChanges<'_>)) -> Arc<Recording> {
    let kinds = vec![KINDS.to_vec(); TABLE + 1];
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing { cap: 256, ..Writing::at(3) }).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..HOURS {
        store.begin_hour(at as Hour, &[]).unwrap();
        for table in 0..TABLE {
            store.status(table, Status::Complete, true, Some(at as Hour), true).unwrap();
        }
        let read = !unread(at);
        let status = if read { Status::Complete } else { gap(&mut store) };
        store.status(TABLE, status, read, read.then_some(at as Hour), true).unwrap();
        if read {
            hour(at, &mut store.table(TABLE).unwrap());
        }
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    Arc::new(Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds })
}

fn unread(at: u32) -> bool {
    (300..330).contains(&at)
}

fn quiet(at: u32, changes: &mut TableChanges<'_>) {
    if at == 0 {
        for slot in 0..8 {
            changes.insert_at(slot, 100 + u128::from(slot)).unwrap();
            changes.write(slot, 0, Cell::I32(0)).unwrap();
            changes.write(slot, 2, Cell::OptionalI64(Some(0))).unwrap();
            for column in [3, 4] {
                changes.write(slot, column, Cell::I64(0)).unwrap();
            }
        }
    }
    for slot in 0..5 {
        changes.write(slot, 0, Cell::I32((at * 10 + slot) as i32)).unwrap();
    }
    changes.offer(5, 3, Cell::I64(i64::from(at / 100))).unwrap();
    changes.write(6, 2, Cell::OptionalI64(Some(i64::from(at % 2)))).unwrap();
    for slot in 0..2 {
        changes.offer(slot, 2, Cell::OptionalI64(Some(i64::from(at / 10 + slot * 1000)))).unwrap();
    }
    changes.offer(7, 4, Cell::I64(i64::from(at / 24))).unwrap();
}

fn churn(at: u32, changes: &mut TableChanges<'_>) {
    quiet(at, changes);
    if at >= 25
        && let Some(slot) = changes.slot(1000 + u128::from(at - 25))
    {
        changes.delete(slot).unwrap();
    }
    if born(at) {
        let slot = changes.insert(1000 + u128::from(at)).unwrap();
        changes.write(slot, 0, Cell::I32(at as i32)).unwrap();
        changes.write(slot, 3, Cell::I64(-i64::from(at))).unwrap();
    }
}

fn appends(at: u32, changes: &mut TableChanges<'_>) {
    quiet(at, changes);
    if born(at) {
        let slot = changes.insert(1000 + u128::from(at)).unwrap();
        changes.write(slot, 0, Cell::I32(at as i32)).unwrap();
        changes.write(slot, 3, Cell::I64(-i64::from(at))).unwrap();
    }
}

fn born(at: u32) -> bool {
    at.is_multiple_of(10) && at + 25 < HOURS && !unread(at)
}

struct Played {
    stats: StepStats,
    message: Option<Message>,
    from: Hour,
    head: Hour,
}

impl Played {
    fn moved(&self) -> bool {
        self.from <= self.head
    }

    fn answers(&self) -> Vec<(u32, Hour, Answer)> {
        self.message.iter().flat_map(|message| decode(&message.bytes).unwrap().answers.into_iter().map(|(id, hour, body)| (id, hour, Answer::parse(body).unwrap())).collect::<Vec<_>>()).collect()
    }
}

struct Harness {
    pages: Arc<Recording>,
    playhead: Option<Arc<Playhead>>,
    follow: Follow,
    consumer: playhead::Consumer,
    played: mpsc::Receiver<Played>,
    head: Hour,
    seen: Hour,
    every: Hour,
    reached: Hour,
    awaiting: bool,
    plain: Arc<std::sync::atomic::AtomicBool>,
}

fn harness(pages: &Arc<Recording>) -> Harness {
    stepping(pages, 24)
}

fn stepping(pages: &Arc<Recording>, every: Hour) -> Harness {
    stepping_with(pages, every, Timeline::whole(&**pages).unwrap(), Playhead::log_reads)
}

fn stepping_with(pages: &Arc<Recording>, every: Hour, timeline: Timeline, logs: impl FnOnce(&Playhead) -> LogReads) -> Harness {
    let follow = Follow::new(0);
    let (send, played) = mpsc::channel();
    let heads = Arc::new(Mutex::new((0, 0)));
    let told = Arc::clone(&heads);
    let sink: Sink = Box::new(move |stats: &StepStats, message: Option<Message>| {
        let (from, head) = *told.lock().unwrap();
        let _ = send.send(Played { stats: *stats, message, from, head });
    });
    let mut inner = follow.hook(sink);
    let mut before: Option<Hour> = None;
    let hook: playhead::Hook = Box::new(move |step, lanes| {
        let head = lanes.get(TABLE).and_then(Lane::head).unwrap_or(step.last());
        let from = if step.reset() { head } else { before.map_or(step.first(), |before| before + 1) };
        before = Some(head);
        *heads.lock().unwrap() = (from, head);
        inner(step, lanes);
    });
    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 100, ring: RING, speed: f64::from(every), rate: 1.0, ..Settings::default() };
    let playhead = Arc::new(Playhead::new(Arc::clone(pages) as Arc<dyn Pages>, timeline, settings, Some(hook)).unwrap());
    let reader = playhead.declare(Vec::new(), None).unwrap();
    let consumer = playhead.consumer(reader);
    let weak = Arc::downgrade(&playhead);
    let plain = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stripped = Arc::clone(&plain);
    follow.bind(move |tables: Vec<playhead::Declared>, _joining| {
        let tables = if stripped.load(std::sync::atomic::Ordering::Relaxed) { tables.into_iter().map(|table| playhead::Declared { passing: false, ..table }).collect() } else { tables };
        weak.upgrade().map_or(Ok(()), |playhead| playhead.redeclare(reader, tables))
    }, logs(playhead.as_ref())).unwrap();
    Harness { pages: Arc::clone(pages), playhead: Some(playhead), follow, consumer, played, head: 0, seen: Hour::MIN, every, reached: Hour::MIN, awaiting: false, plain }
}

impl Harness {
    fn playhead(&self) -> &Playhead {
        self.playhead.as_ref().expect("playing")
    }

    fn seek(&mut self, hour: Hour) {
        self.playhead().seek(hour);
        self.head = hour;
        self.seen = Hour::MIN;
        self.reached = Hour::MIN;
        self.awaiting = true;
    }

    fn play(&mut self, to: Hour) -> Vec<Played> {
        let mut out = Vec::new();
        loop {
            if !self.awaiting && self.seen >= self.head {
                if self.head < to || self.reached < to {
                    self.head = (self.head + self.every).min(if self.head < to { to } else { HOURS as Hour - 1 });
                }
                if self.seen >= self.head && self.reached >= to {
                    return out;
                }
            }
            let mut played = None;
            let playhead = self.playhead.as_ref().expect("playing");
            playhead
                .drive_until(&mut self.consumer, self.head, || match self.played.try_recv() {
                    Ok(item) => {
                        played = Some(item);
                        std::ops::ControlFlow::Break(Ok(()))
                    }
                    Err(mpsc::TryRecvError::Empty) => std::ops::ControlFlow::Continue(()),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        std::ops::ControlFlow::Break(Err("step hook channel disconnected".into()))
                    }
                })
                .unwrap_or_else(|error| panic!("no step at head {} (the last seen ends at {}, Countries at {}, the newest built at {:?}, playing to {to}): {error}", self.head, self.seen, self.reached, playhead.newest()));
            let played = played.unwrap();
            self.awaiting &= !played.stats.reset;
            if !self.awaiting {
                (self.seen, self.reached) = (played.stats.last, played.head);
            }
            out.push(played);
        }
    }

    fn fresh(&self, query: &Query, hour: Hour) -> Answer {
        let mut lane = Lane::new(Arc::clone(&self.pages) as Arc<dyn Pages>, 0, TABLE, &(0..KINDS.len()).filter(|&column| column != 1).collect::<Vec<_>>(), false).unwrap();
        lane.seek(hour).unwrap();
        Answer::parse(&answer_at(query, &lane).unwrap()).unwrap()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(playhead) = self.playhead.take()
            && let Ok(playhead) = Arc::try_unwrap(playhead)
        {
            playhead.finish();
        }
    }
}

fn keys(keys: &[u128], columns: &[usize]) -> Query {
    Query::Cells { table: TABLE, columns: columns.to_vec(), rows: Rows::Keys(keys.to_vec()) }
}

fn covers(played: &Played, hour: Hour) -> bool {
    played.from <= hour && hour <= played.head
}

fn moves_slot_5(played: &Played) -> bool {
    (100..=played.head).step_by(100).any(|hour| !unread(hour as u32) && covers(played, hour))
}

#[test]
fn a_query_is_evaluated_only_when_a_step_touched_its_columns_and_keys() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    harness.follow.follow(1, keys(&[105], &[3])).unwrap();
    harness.follow.follow(2, keys(&[100], &[3])).unwrap();
    harness.seek(0);
    let played = harness.play(280);
    let first = played.iter().position(|played| played.answers().len() == 2).expect("both answered once their lane held them");
    for played in played[first + 1..].iter().filter(|played| played.head < 300) {
        let moved = covers(played, 100) || covers(played, 200);
        assert_eq!(played.stats.touched, u32::from(moved), "steps {}..{}", played.stats.first, played.stats.last);
        assert_eq!(played.stats.evaluated, played.stats.touched);
        let ids: Vec<u32> = played.answers().iter().map(|(id, ..)| *id).collect();
        assert_eq!(ids, if moved { vec![1] } else { vec![] });
    }
    for (id, hour, answer) in played.iter().flat_map(Played::answers) {
        let query = if id == 1 { keys(&[105], &[3]) } else { keys(&[100], &[3]) };
        assert_eq!(answer, harness.fresh(&query, hour), "query {id} at hour {hour}");
    }
}

fn assert_claimed_only_where_read(harness: &Harness, query: &Query, played: &[Played]) -> u32 {
    let mut facts: Vec<(Hour, Hour, Answer)> = Vec::new();
    let mut restated = 0;
    for played in played {
        let Some(message) = &played.message else { continue };
        let message = decode(&message.bytes).unwrap();
        if message.reset {
            facts.clear();
        }
        for (_, hour, body) in message.answers {
            let answer = Answer::parse(body).unwrap();
            if facts.last().is_some_and(|last| last.0 == hour) {
                facts.pop();
            } else if let Some(last) = facts.last_mut() {
                restated += u32::from(last.2 == answer);
                last.1 = last.1.min(hour - 1);
            }
            facts.push((hour, hour, answer));
        }
        let through = message.limited.iter().rev().find(|&&(id, _)| id == 7).map_or(message.hour, |&(_, through)| through);
        if let Some(last) = facts.last_mut() {
            last.1 = last.1.max(through);
        }
    }
    for (from, through, answer) in &facts {
        for hour in *from..=*through {
            assert_eq!(answer, &harness.fresh(query, hour), "the answer read at hour {from} is claimed at hour {hour}");
        }
    }
    restated
}

#[test]
fn an_answer_equal_to_the_last_is_claimed_only_at_the_hours_it_was_read() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    let query = keys(&[106], &[2]);
    harness.follow.follow(7, query.clone()).unwrap();
    harness.seek(0);
    let played = harness.play(280);
    let restated = assert_claimed_only_where_read(&harness, &query, &played);
    assert!(restated > 5, "{restated} answers equal to the last were restated at the hour they were read");
}

#[test]
fn a_row_born_and_gone_inside_a_step_is_never_claimed_absent_over_its_life() {
    for plain in [false, true] {
        let pages = record(brief);
        let mut harness = harness(&pages);
        harness.plain.store(plain, std::sync::atomic::Ordering::Relaxed);
        let query = keys(&[1010], &[0]);
        harness.follow.follow(7, query.clone()).unwrap();
        harness.seek(0);
        let played = harness.play(100);
        assert!(assert_claimed_only_where_read(&harness, &query, &played) > 0, "the absent row is restated at the step's end, the lane passing: {}", !plain);
    }
}

#[test]
fn a_table_unread_inside_a_step_is_never_claimed_over_its_unread_hours() {
    let pages = record(quiet);
    let mut harness = stepping(&pages, 48);
    let query = keys(&[100], &[3]);
    harness.follow.follow(7, query.clone()).unwrap();
    harness.seek(0);
    let played = harness.play(400);
    assert!(played.iter().any(|played| played.from < 300 && played.head >= 330), "a step spans the unread hours");
    assert!(assert_claimed_only_where_read(&harness, &query, &played) > 0, "the unchanged answer is restated past the unread hours");
}

#[test]
fn a_reset_evaluates_every_query_once() {
    let pages = record(churn);
    let mut harness = harness(&pages);
    let queries = [
        keys(&[100, 105], &[0, 3]),
        Query::Cells { table: TABLE, columns: vec![0], rows: Rows::All },
        Query::Cells { table: TABLE, columns: vec![3], rows: Rows::Top { column: 0, n: 3 } },
        keys(&[107], &[4]),
    ];
    for (id, query) in queries.iter().enumerate() {
        harness.follow.follow(id as u32, query.clone()).unwrap();
    }
    harness.seek(0);
    harness.play(200);
    harness.seek(900);
    let played = harness.play(948);
    let reset = played.iter().find(|played| played.stats.reset).expect("the seek's reset step");
    assert_eq!((reset.stats.touched, reset.stats.evaluated), (4, 4));
    let message = reset.message.as_ref().expect("a settled message");
    assert!(message.reset && decode(&message.bytes).unwrap().reset);
    for (id, hour, answer) in reset.answers() {
        assert_eq!(answer, harness.fresh(&queries[id as usize], hour), "query {id}");
    }
}

#[test]
fn a_table_not_recorded_answers_not_recorded() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    harness.follow.follow(3, keys(&[100], &[0])).unwrap();
    harness.seek(0);
    let played = harness.play(400);
    let answers: Vec<(Hour, Answer)> = played.iter().flat_map(Played::answers).map(|(_, hour, answer)| (hour, answer)).collect();
    let gap: Vec<&(Hour, Answer)> = answers.iter().filter(|(hour, _)| (300..330).contains(hour)).collect();
    let stopped = played.iter().filter(|played| (300..330).contains(&played.head)).count();
    assert!(stopped > 0, "a step ends while it is not recorded");
    assert_eq!(gap.len(), 1, "one answer while it is not recorded, sent once");
    let reason = gap[0].1.not_recorded.as_deref().expect("not recorded");
    assert_eq!(reason, "Countries not recorded at this hour", "no verb to agree with a plural table");
    for (hour, answer) in &answers {
        if !(300..330).contains(hour) {
            assert_eq!(answer.not_recorded, None, "hour {hour}");
            assert_eq!(answer.values, vec![i64::from(*hour) * 10], "hour {hour}: the value, never a held one or zero");
        }
    }
}

#[test]
fn a_table_not_available_answers_its_recorded_reason_in_words_never_its_id() {
    const WHY: &str = "the game's navies could not be followed";
    let pages = record_with(|store| Status::Unavailable { reason: store.intern(WHY.as_bytes()).unwrap() }, quiet);
    let said = format!("Countries not available at this hour: {WHY}");
    let mut harness = harness(&pages);
    let query = keys(&[100], &[0]);
    harness.follow.follow(3, query.clone()).unwrap();
    harness.seek(0);
    let mut played = harness.play(400);
    harness.seek(310);
    played.extend(harness.play(340));
    let reasons: Vec<String> = played.iter().flat_map(Played::answers).filter(|(_, hour, _)| (300..330).contains(hour)).filter_map(|(_, _, answer)| answer.not_recorded).collect();
    assert!(reasons.len() >= 2, "played into the stretch and sought inside it: {reasons:?}");
    for reason in &reasons {
        assert_eq!(reason, &said, "the recorded reason's text, never its id");
    }
    assert_eq!(harness.fresh(&query, 310).not_recorded.as_deref(), Some(said.as_str()), "a lane sought into the stretch says the same");
}

#[test]
fn seventy_thousand_texts_are_each_read_back_by_their_own_row() {
    let mut table = crate::answer::Table::default();
    table.clear(1, false);
    for row in 0..70_000u32 {
        table.row(u128::from(row));
        table.cell(0, State::Missing);
        if row % 2 == 0 { table.explain(&format!("reason {row}")) } else { table.name(&format!("name {row}")) }
    }
    let mut body = Vec::new();
    table.encode(&mut body);
    let answer = Answer::parse(&body).unwrap();
    assert_eq!(answer.texts.len(), 70_000);
    for row in 0..70_000usize {
        let (reason, name) = if row % 2 == 0 { (Some(format!("reason {row}")), None) } else { (None, Some(format!("name {row}"))) };
        assert_eq!((answer.reason(row), answer.name(row)), (reason.as_deref(), name.as_deref()), "row {row}");
    }
}

#[test]
fn unfollow_at_a_step_boundary_sends_nothing_more() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    harness.follow.follow(1, keys(&[100], &[0])).unwrap();
    harness.follow.follow(2, keys(&[101], &[0])).unwrap();
    harness.seek(0);
    let before = harness.play(200);
    assert!(before.iter().flat_map(Played::answers).any(|(id, ..)| id == 1));
    harness.follow.unfollow(1).unwrap();
    let after = harness.play(400);
    let ids = |played: &Played| played.answers().iter().map(|(id, ..)| *id).collect::<Vec<u32>>();
    let steps = || after.iter().map(|played| (played.from, played.head, ids(played))).collect::<Vec<_>>();
    let named: Vec<usize> = after.iter().enumerate().filter(|&(_, played)| ids(played).contains(&1)).map(|(at, _)| at).collect();
    assert!(named.iter().all(|&at| at < RING), "id 1 answered after the {RING} steps built before the unfollow: steps {named:?} of (from, head, ids) {:?}", steps());
    let mut unrecorded = before.iter().rev().find(|played| played.moved()).is_some_and(|played| unread(played.head as u32));
    for played in after.iter().filter(|played| played.moved()) {
        let now = unread(played.head as u32);
        let changed = !(now && unrecorded);
        assert_eq!(ids(played).contains(&2), changed, "id 2 at the step of hours {}..{}: (from, head, ids) {:?}", played.from, played.head, steps());
        unrecorded = now;
    }
    assert_eq!(harness.follow.held().queries, 1);
}

#[test]
fn a_rank_reorders_only_when_its_column_was_written() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    let rank = Query::Cells { table: TABLE, columns: vec![3], rows: Rows::Top { column: 3, n: 3 } };
    harness.follow.follow(9, rank.clone()).unwrap();
    harness.seek(0);
    let played = harness.play(280);
    let first = played.iter().position(|played| !played.answers().is_empty()).expect("a first answer");
    for played in played[first + 1..].iter().filter(|played| played.head < 300) {
        let written = covers(played, 100) || covers(played, 200);
        assert_eq!(played.stats.evaluated, u32::from(written), "steps {}..{}", played.stats.first, played.stats.last);
    }
    let answers: Vec<(Hour, Answer)> = played.iter().flat_map(Played::answers).map(|(_, hour, answer)| (hour, answer)).filter(|(hour, _)| *hour < 300).collect();
    let (hour, top) = answers.last().unwrap();
    assert_eq!(top, &harness.fresh(&rank, *hour));
    assert_eq!(top.keys, vec![105, 100, 101], "slot 5 first, then ties by key");
    assert_eq!(top.values, vec![2, 0, 0]);
}

#[test]
fn a_log_pushes_exactly_the_rows_inserted_since_its_last_push() {
    let pages = record(appends);
    let mut harness = harness(&pages);
    harness.follow.follow(4, Query::Cells { table: TABLE, columns: vec![0, 3], rows: Rows::Appended }).unwrap();
    harness.seek(0);
    let played = harness.play(1000);
    let mut rows: Vec<(u128, Hour, i64, i64)> = Vec::new();
    for (id, hour, answer) in played.iter().flat_map(Played::answers) {
        assert_eq!(id, 4);
        let hours = answer.hours.clone().expect("rows appended");
        for (row, &born) in hours.iter().enumerate() {
            assert!(born <= hour);
            assert_eq!(answer.states[row * 2..row * 2 + 2], [State::Value, State::Value]);
            rows.push((answer.keys[row], born, answer.values[row * 2], answer.values[row * 2 + 1]));
        }
    }
    let first = rows.first().expect("rows").1;
    let last = played.last().expect("steps").head;
    let wanted: Vec<(u128, Hour, i64, i64)> = (first..=last).filter(|&at| born(at as u32)).map(|at| (1000 + at as u128, at, i64::from(at), -i64::from(at))).collect();
    assert_eq!(rows, wanted);
}

static POWER: Series = Series {
    kind: "test-power",
    table: TableId::Countries,
    key: Key::Country,
    parts: None,
    measures: &[Measure { name: "power", column: Column::Cell("political_power_raw"), decimals: 5, aggregates: 0, carry: Carry::None, rank: 0, rank_zero: dashboard_declare::RankZero::Keep }],
    drops: false,
    absent_none: false,
};

static POWER_NONE: Series = Series {
    kind: "test-power-none",
    table: TableId::Countries,
    key: Key::Country,
    parts: None,
    measures: &[Measure { name: "power", column: Column::Cell("political_power_raw"), decimals: 5, aggregates: 0, carry: Carry::None, rank: 0, rank_zero: dashboard_declare::RankZero::Keep }],
    drops: false,
    absent_none: true,
};

#[test]
fn a_series_reads_its_tags_holder_at_the_head() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    let run = |first, last, slot, id| HolderRun { first, last, holder: Holder::Found { slot, id }, starts_segment: true };
    let mut holders = Holders::default();
    holders.tags.insert("AAA".into(), vec![run(0, 149, 0, 100), run(150, HOURS as Hour - 1, 1, 101)]);
    holders.tags.insert("BBB".into(), vec![HolderRun { first: 0, last: HOURS as Hour - 1, holder: Holder::Unresolved, starts_segment: true }]);
    let timeline = Stretches { id: 1, first: 0, last: HOURS as Hour - 1, stretches: vec![Stretch { run: 0, first: 0, last: HOURS as Hour - 1, join: Join::Start }], unheld: Vec::new() };
    let series = Query::Series {
        series: &POWER,
        measure: &POWER.measures[0],
        countries: vec!["AAA".into(), "BBB".into(), "CCC".into()],
        mode: SeriesMode::Keyed(CoverageScope::Table),
        identity: Identity { holders: Arc::new(holders), timeline: Arc::new(timeline) },
    };
    harness.follow.follow(5, series.clone()).unwrap();
    harness.seek(0);
    let played = harness.play(250);
    let answers: Vec<(Hour, Answer)> = played.iter().flat_map(Played::answers).map(|(_, hour, answer)| (hour, answer)).collect();
    for (hour, answer) in &answers {
        assert_eq!(answer, &harness.fresh(&series, *hour), "hour {hour}");
        let holder = if *hour < 150 { (100, i64::from(*hour) / 10) } else { (101, i64::from(*hour) / 10 + 1000) };
        assert_eq!((answer.keys[0], answer.values[0], answer.states[0]), (holder.0, holder.1, State::Value), "hour {hour}");
        assert_eq!(answer.reason(1), Some("No link resolves the country holding BBB"));
        assert_eq!(answer.reason(2), Some("CCC is held by no country"));
        assert_eq!(answer.states[1..], [State::NoRow, State::NoRow]);
    }
    let moved = played.iter().find(|played| covers(played, 150)).expect("the step the holder changes in");
    assert_eq!(moved.answers().len(), 1, "a new holder is answered in the step it takes the tag");
    let top = Query::series_top(&POWER, &POWER.measures[0], 1, series_identity(&series));
    harness.follow.follow(6, top.clone()).unwrap();
    harness.play(260);
    harness.seek(0);
    let played: Vec<Played> = harness.play(120).into_iter().skip_while(|played| !played.stats.reset).collect();
    let answers: Vec<(u32, Hour, Answer)> = played.iter().flat_map(Played::answers).collect();
    assert!(answers.iter().any(|(id, ..)| *id == 5) && answers.iter().any(|(id, ..)| *id == 6), "both answered after the seek");
    for (id, hour, answer) in &answers {
        assert_eq!(answer, &harness.fresh(if *id == 5 { &series } else { &top }, *hour), "query {id} at hour {hour} after the seek back");
        if *id == 6 {
            assert_eq!(answer.not_recorded.as_deref(), Some("Countries holds a row of a country not known at this hour"), "rows no tag holds refuse the top at {hour}");
            continue;
        }
        assert_eq!((answer.keys[0], answer.values[0]), (100, i64::from(*hour) / 10), "query {id} at hour {hour}: AAA's first holder");
    }
}

fn series_identity(query: &Query) -> Identity {
    let Query::Series { identity, .. } = query else { panic!("a series") };
    identity.clone()
}

#[test]
fn a_top_breaks_a_tie_at_its_last_place_by_tag_as_the_race_does() {
    let pages = record(quiet);
    let held = |slot, id| vec![HolderRun { first: 0, last: HOURS as Hour - 1, holder: Holder::Found { slot, id }, starts_segment: true }];
    let timeline = Stretches { id: 1, first: 0, last: HOURS as Hour - 1, stretches: vec![Stretch { run: 0, first: 0, last: HOURS as Hour - 1, join: Join::Start }], unheld: Vec::new() };
    let identity = |tags: &[(&str, u32, u32)]| {
        let mut holders = Holders::default();
        for &(tag, slot, id) in tags {
            holders.tags.insert(tag.into(), held(slot, id));
        }
        Identity { holders: Arc::new(holders), timeline: Arc::new(timeline.clone()) }
    };
    let partly = identity(&[("MMM", 1, 101), ("ZZZ", 2, 102), ("AAA", 3, 103), ("NNN", 9, 999)]);
    let mut first = harness(&pages);
    let unknown = Query::series_top(&POWER, &POWER.measures[0], 2, partly.clone());
    let unknown_none = Query::series_top(&POWER_NONE, &POWER_NONE.measures[0], 2, partly);
    first.follow.follow(5, unknown.clone()).unwrap();
    first.follow.follow(6, unknown_none.clone()).unwrap();
    first.seek(0);
    let played = first.play(50);
    for (id, query) in [(5, &unknown), (6, &unknown_none)] {
        let (_, hour, answer) = played.iter().flat_map(Played::answers).filter(|(at, _, _)| *at == id).next_back().expect("an answer");
        assert_eq!(answer, first.fresh(query, hour));
        assert_eq!(answer.not_recorded.as_deref(), Some("Countries holds a row of a country not known at this hour"), "a row no tag holds refuses the top, never drops out");
    }

    let whole = identity(&[("MMM", 1, 101), ("ZZZ", 2, 102), ("AAA", 3, 103), ("NNN", 9, 999), ("CCC", 0, 100), ("BBB", 4, 104), ("YYY", 5, 105), ("ZZY", 6, 106), ("YYZ", 7, 107)]);
    let mut harness = harness(&pages);
    let top = Query::series_top(&POWER, &POWER.measures[0], 2, whole.clone());
    let least = Query::series_top(&POWER, &POWER.measures[0], 2, whole.clone()).least();
    let none_top = Query::series_top(&POWER_NONE, &POWER_NONE.measures[0], 2, whole.clone());
    let none_least = Query::series_top(&POWER_NONE, &POWER_NONE.measures[0], 3, whole).least();
    harness.follow.follow(7, top.clone()).unwrap();
    harness.follow.follow(8, least.clone()).unwrap();
    harness.follow.follow(9, none_top.clone()).unwrap();
    harness.follow.follow(10, none_least.clone()).unwrap();
    harness.seek(0);
    let played = harness.play(50);
    let last = |id: u32| played.iter().flat_map(Played::answers).filter(|(at, _, _)| *at == id).next_back().expect("an answer");
    let cases: [(u32, &Query, &[&str], &str); 4] = [
        (7, &top, &["MMM", "CCC"], "value first"),
        (8, &least, &["AAA", "BBB"], "the least first, a tie at the last place by tag in byte order, never key"),
        (9, &none_top, &["MMM", "CCC"], "value first"),
        (10, &none_least, &["AAA", "BBB", "NNN"], "a holder with no row ranks at zero, a tie by tag"),
    ];
    for (id, query, wanted, why) in cases {
        let (_, hour, answer) = last(id);
        assert_eq!(answer, harness.fresh(query, hour));
        let names: Vec<&str> = (0..answer.keys.len()).filter_map(|row| answer.name(row)).collect();
        assert_eq!(names, wanted, "{why}");
    }
    let (_, _, answer) = last(10);
    assert_eq!((answer.keys[2], answer.values[2], answer.states[2]), (9, 0, State::Value));
}

static PARTS: Series = Series {
    kind: "test-parts",
    table: TableId::Countries,
    key: Key::Country,
    parts: Some(Parts { bits: 2, id: None }),
    measures: &[Measure { name: "slot", column: Column::Cell("canonical_slot"), decimals: 0, aggregates: 0, carry: Carry::None, rank: 0, rank_zero: dashboard_declare::RankZero::Keep }],
    drops: false,
    absent_none: false,
};

#[test]
fn a_followed_owners_parts_are_its_rows_in_key_order_or_its_top_parts() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    let held = |id| vec![HolderRun { first: 0, last: HOURS as Hour - 1, holder: Holder::Found { slot: 0, id }, starts_segment: true }];
    let mut holders = Holders::default();
    holders.tags.insert("AAA".into(), held(25));
    holders.tags.insert("BBB".into(), held(26));
    let timeline = Stretches { id: 1, first: 0, last: HOURS as Hour - 1, stretches: vec![Stretch { run: 0, first: 0, last: HOURS as Hour - 1, join: Join::Start }], unheld: Vec::new() };
    let identity = Identity { holders: Arc::new(holders), timeline: Arc::new(timeline) };
    let top = Query::Series { series: &PARTS, measure: &PARTS.measures[0], countries: vec!["AAA".into(), "BBB".into()], mode: SeriesMode::Parts { top: Some(2) }, identity: identity.clone() };
    let all = Query::Series { series: &PARTS, measure: &PARTS.measures[0], countries: vec!["BBB".into()], mode: SeriesMode::Parts { top: None }, identity };
    harness.follow.follow(1, top.clone()).unwrap();
    harness.follow.follow(2, all.clone()).unwrap();
    harness.seek(0);
    let played = harness.play(400);
    let mut answered = 0;
    for (id, hour, answer) in played.iter().flat_map(Played::answers) {
        let query = if id == 1 { &top } else { &all };
        assert_eq!(answer, harness.fresh(query, hour), "query {id} at hour {hour}");
        if unread(hour as u32) {
            assert!(answer.not_recorded.is_some());
            continue;
        }
        let at = i64::from(hour) * 10;
        let wanted: Vec<(u128, i64, &str)> = match id {
            1 => vec![(103, at + 3, "AAA"), (102, at + 2, "AAA"), (104, at + 4, "BBB"), (105, 0, "BBB")],
            _ => vec![(104, at + 4, "BBB"), (105, 0, "BBB"), (106, 0, "BBB"), (107, 0, "BBB")],
        };
        let got: Vec<(u128, i64, &str)> = (0..answer.keys.len()).map(|row| (answer.keys[row], answer.values[row], answer.name(row).unwrap())).collect();
        assert_eq!(got, wanted, "query {id} at hour {hour}");
        answered += 1;
    }
    assert!(answered > 10, "{answered} answers compared");
    let one = Query::Series { series: &PARTS, measure: &PARTS.measures[0], countries: vec!["AAA".into()], mode: SeriesMode::Keyed(CoverageScope::Table), identity: nobody() };
    assert!(harness.follow.follow(3, one).is_err(), "a sub-keyed series is followed by its owners' parts");
}

#[test]
fn a_query_of_a_text_column_is_refused_by_name() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    harness.follow.follow(6, keys(&[100], &[1])).unwrap();
    harness.seek(0);
    let played = harness.play(48);
    let answers: Vec<Answer> = played.iter().flat_map(Played::answers).map(|(.., answer)| answer).collect();
    assert_eq!(answers.len(), 1, "refused once");
    assert_eq!(answers[0].not_recorded.as_deref(), Some("Countries's tag is a text column, which a followed query does not read"));
}

struct Paper {
    follow: Follow,
    held: Vec<bool>,
    anchors: Mutex<Vec<crate::registry::Anchor>>,
}

impl crate::registry::Hosted for Paper {
    fn follow(&self) -> &Follow {
        &self.follow
    }

    fn lacks(&self, table: usize) -> Option<String> {
        crate::registry::lacking(table, self.held.get(table).copied().unwrap_or(false))
    }

    fn anchor(&self, anchor: crate::registry::Anchor) {
        self.anchors.lock().unwrap().push(anchor);
    }
}

struct Papers(BTreeMap<String, Arc<Paper>>);

impl crate::registry::Playheads for Papers {
    fn open(&self, path: &std::path::Path, _branch: u64, clock: i32) -> Result<Arc<dyn crate::registry::Hosted>, String> {
        let path = path.to_string_lossy();
        self.0.get(&if clock == 0 { path.into_owned() } else { format!("{path}@{clock}") }).map(|paper| Arc::clone(paper) as Arc<dyn crate::registry::Hosted>).ok_or_else(|| "No playhead for this recording".to_owned())
    }
}

fn paper() -> Arc<Paper> {
    Arc::new(Paper { follow: Follow::new(0), held: vec![true; TABLE + 1], anchors: Mutex::default() })
}

fn nobody() -> Identity {
    Identity { holders: Arc::new(Holders::default()), timeline: Arc::new(Stretches { id: 1, first: 0, last: 0, stretches: Vec::new(), unheld: Vec::new() }) }
}

fn follow_request(id: u32, kind: &str, measure: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "branchId": "1", "clock": 0, "query": { "product": "series-state", "params": {
        "branchId": "1", "family": { "kind": kind, "measure": measure }, "reads": [{ "country": "GER", "hour": 5 }, { "country": "SOV", "hour": 5 }, { "country": "GER", "hour": 9 }] } } })
}

#[test]
fn a_follow_for_a_table_the_playhead_lacks_answers_not_available_and_makes_no_reader() {
    let held = paper();
    let registry = crate::registry::Registry::new(Some(Arc::new(Papers(BTreeMap::from([("a".to_owned(), Arc::clone(&held))])))));
    let path = std::path::Path::new("a");
    let reply = registry.answer(path, "follow", &follow_request(1, "country-stability", "stability"), |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel"))).unwrap();
    assert_eq!(reply["available"], false);
    assert_eq!(reply["reason"], "CountryStability is not in this playhead's files (a derived table, until format 12)");
    assert!(held.follow.declared().unwrap().is_empty(), "no reader made");
    let none = registry.answer(std::path::Path::new("b"), "follow", &follow_request(2, "country-measure", "politicalPower"), |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel"))).unwrap();
    assert_eq!((none["available"].clone(), none["reason"].clone()), (serde_json::json!(false), serde_json::json!("No playhead for this recording")));
    let alone = crate::registry::Registry::new(None).answer(path, "follow", &follow_request(3, "country-measure", "politicalPower"), |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel"))).unwrap();
    assert_eq!(alone["available"], false, "a process with no playheads follows nothing");
    let mut elsewhere = follow_request(5, "country-measure", "politicalPower");
    elsewhere["query"] = serde_json::json!({ "head": "campaign-clock", "params": { "branchId": "2", "firstHour": 200, "lastHour": 209 } });
    let refused = registry.answer(path, "follow", &elsewhere, |_| Ok(nobody()), Box::new(|_| panic!("a head naming another branch is never built"))).unwrap();
    assert_eq!((refused["available"].clone(), refused["reason"].clone()),
        (serde_json::json!(false), serde_json::json!("The followed query names branch 2, not the followed branch 1")));
    let followed = registry.answer(path, "follow", &follow_request(4, "country-measure", "politicalPower"), |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel"))).unwrap();
    assert_eq!(followed["available"], true);
    assert_eq!(held.follow.declared().unwrap(), vec![playhead::Declared { table: TABLE, columns: vec![2], keyed: false, passing: true, hourly: false }]);
    assert_eq!(registry.followed(), 1);
}

#[test]
fn two_tracks_of_one_timeline_at_different_anchors_each_follow_their_own_hours() {
    let (calendar, anchored) = (paper(), paper());
    let registry = crate::registry::Registry::new(Some(Arc::new(Papers(BTreeMap::from([("a".to_owned(), Arc::clone(&calendar)), ("a@1000".to_owned(), Arc::clone(&anchored))])))));
    let path = std::path::Path::new("a");
    let mut asked = follow_request(1, "country-measure", "politicalPower");
    registry.answer(path, "follow", &asked, |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel"))).unwrap();
    asked["id"] = 2.into();
    asked["clock"] = 1000.into();
    registry.answer(path, "follow", &asked, |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel"))).unwrap();
    assert_eq!((calendar.follow.declared().unwrap().len(), anchored.follow.declared().unwrap().len()), (1, 1), "each mapping's playhead reads its own follow");
    let anchor = |clock: i32, hour: i32| serde_json::json!({ "branchId": "1", "clock": clock, "seek": 12, "frameRate": 120, "hour": hour, "playing": false, "speed": 24.0, "sought": true });
    registry.answer(path, "follow-clock", &anchor(0, 10), |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel"))).unwrap();
    registry.answer(path, "follow-clock", &anchor(1000, 1010), |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel"))).unwrap();
    assert_eq!(calendar.anchors.lock().unwrap()[0].seek, 12);
    assert_eq!(calendar.anchors.lock().unwrap()[0].frame_rate, 120.0);
    let hours = |paper: &Paper| paper.anchors.lock().unwrap().iter().map(|anchor| anchor.hour).collect::<Vec<_>>();
    assert_eq!((hours(&calendar), hours(&anchored)), (vec![10], vec![1010]), "each at the hour its track shows");
}

#[test]
fn unfollow_during_preparation_cannot_reinstall_a_removed_registration() {
    let host = paper();
    let registry = crate::registry::Registry::new(Some(Arc::new(Papers(BTreeMap::from([("a".to_owned(), host)])))));
    let path = std::path::Path::new("a");
    let power = |id| follow_request(id, "country-measure", "politicalPower");
    let (entered, started) = mpsc::channel();
    let (open, gate) = mpsc::channel();
    std::thread::scope(|threads| {
        let registry = &registry;
        let slow = threads.spawn(move || registry.answer(path, "follow", &power(1), |_| {
            entered.send(()).unwrap();
            gate.recv().unwrap();
            Ok(nobody())
        }, Box::new(|_| unreachable!())).unwrap());
        started.recv().unwrap();
        registry.answer(path, "unfollow", &serde_json::json!({ "id": 1 }), |_| unreachable!(), Box::new(|_| unreachable!())).unwrap();
        let fast = registry.answer(path, "follow", &power(2), |_| Ok(nobody()), Box::new(|_| unreachable!())).unwrap();
        assert_eq!(fast["available"], true);
        open.send(()).unwrap();
        assert_eq!(slow.join().unwrap()["available"], false);
    });
    assert_eq!(registry.followed(), 1);
}

#[test]
fn the_requests_a_widget_sends_parse_into_its_query() {
    match crate::parse(&follow_request(0, "country-measure", "commandPower")["query"], &nobody()).unwrap() {
        Query::Series { series, measure, countries, mode: SeriesMode::Keyed(CoverageScope::Table), .. } => assert_eq!((series.kind, measure.name, countries), ("country-measure", "commandPower", vec!["GER".into(), "SOV".into()])),
        _ => panic!("a series"),
    }
    let slot = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "country-measure", "measure": "commandPower" }, "reads": [{ "country": 7, "hour": 5 }, { "country": "GER", "hour": 5 }] } });
    match crate::parse(&slot, &nobody()).unwrap() {
        Query::Series { countries, .. } => assert_eq!(countries, vec![dashboard_query::identity::Country::Slot(7), "GER".into()]),
        _ => panic!("a series"),
    }
    let army = serde_json::json!({ "product": "army-size", "params": { "branchId": "1", "countries": [], "top": 10, "measure": "divisions", "countFor": "owner", "byType": false } });
    match crate::parse(&army, &nobody()).unwrap() {
        Query::Series { series, measure, mode: SeriesMode::Top { n: 10, least: false, .. }, .. } => assert_eq!((series.kind, measure.name), ("army-totals", "ownerDivisions")),
        _ => panic!("a series"),
    }
    let typed = serde_json::json!({ "product": "army-size", "params": { "branchId": "1", "countries": ["GER"], "measure": "battalions", "countFor": "owner", "byType": true } });
    assert_eq!(crate::parse(&typed, &nobody()).err().unwrap().to_string(), "Army sizes by unit type are not followed at the head");
    let air = serde_json::json!({ "product": "air-force-size", "params": { "branchId": "1", "countries": [], "top": 10, "measure": "wings", "ownership": "effective" } });
    match crate::parse(&air, &nobody()).unwrap() {
        Query::Series { series, measure, mode: SeriesMode::Top { n: 10, least: false, .. }, .. } => assert_eq!((series.kind, measure.name), ("air-force-totals", "effectiveWings")),
        _ => panic!("a series"),
    }
    let navy = serde_json::json!({ "product": "navy-size", "params": { "branchId": "1", "countries": ["GER"], "top": null, "measure": "strength", "ownership": "logical", "grouping": "type" } });
    match crate::parse(&navy, &nobody()).unwrap() {
        Query::Series { series, measure, countries, mode: SeriesMode::Keyed(CoverageScope::Table), .. } => assert_eq!((series.kind, measure.name, countries), ("navy-totals", "logicalStrength", vec!["GER".into()])),
        _ => panic!("a series"),
    }
    let unknown = serde_json::json!({ "product": "navy-size", "params": { "branchId": "1", "countries": ["GER"], "measure": "planes", "ownership": "roster" } });
    assert_eq!(crate::parse(&unknown, &nobody()).err().unwrap().to_string(), "No navy-size measure planes counted for roster");
    let other = serde_json::json!({ "product": "series-rank", "params": {} });
    assert_eq!(crate::parse(&other, &nobody()).err().unwrap().to_string(), "series-rank is not followed at the head");
    let parts = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "resource-balance", "measure": "supply" }, "reads": [{ "country": "GER", "hour": 5 }], "top": 3 } });
    match crate::parse(&parts, &nobody()).unwrap() {
        Query::Series { series, measure, countries, mode: SeriesMode::Parts { top: Some(3) }, .. } => assert_eq!((series.kind, measure.name, countries), ("resource-balance", "supply", vec!["GER".into()])),
        _ => panic!("a series"),
    }
    let one = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "country-resources", "measure": "used" }, "reads": [{ "country": "GER", "hour": 5, "part": "2" }] } });
    assert!(crate::parse(&one, &nobody()).is_err(), "one part is not followed");
    let territory = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "territory", "measure": "controlledStates" }, "top": 10, "hour": 5 } });
    match crate::parse(&territory, &nobody()).unwrap() {
        Query::Series { series, measure, mode: SeriesMode::Top { n: 10, least: false, .. }, .. } => assert_eq!((series.kind, measure.name), ("territory", "controlledStates")),
        _ => panic!("a ranked series"),
    }
    let unranked = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "territory", "measure": "ownedArea" }, "hour": 5 } });
    assert!(crate::parse(&unranked, &nobody()).is_ok(), "a series without countries or a top follows the full population");
    let ranked = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "country-game-scores", "measure": "total" }, "top": 10 } });
    match crate::parse(&ranked, &nobody()).unwrap() {
        Query::Series { series, measure, mode: SeriesMode::Top { n: 10, least: false, .. }, .. } => assert_eq!((series.kind, measure.name), ("country-game-scores", "total")),
        _ => panic!("a ranked series"),
    }
    let chosen_top = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "country-game-scores", "measure": "total" }, "reads": [{ "country": "GER", "hour": 0 }], "top": 1 } });
    match crate::parse(&chosen_top, &nobody()).unwrap() {
        Query::Series { countries, mode: SeriesMode::Top { n: 1, least: false, .. }, .. } => assert_eq!(countries, vec!["GER".into()]),
        _ => panic!("a top of named countries ranks them"),
    }
    let world = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "world-tension", "measure": "tension" }, "top": 3 } });
    assert!(crate::parse(&world, &nobody()).is_err(), "a singleton is never ranked");
    let tension = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "world-tension", "measure": "tension" }, "reads": [{ "country": "World", "hour": 0 }] } });
    assert!(matches!(crate::parse(&tension, &nobody()).unwrap(), Query::Series { mode: SeriesMode::Keyed(CoverageScope::Table), .. }));
    let members: Vec<serde_json::Value> = (0..1_500).map(|at| serde_json::json!({ "country": format!("T{at:04}"), "hour": 0 })).collect();
    let many = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "territory", "measure": "controlledStates" }, "reads": members } });
    match crate::parse(&many, &nobody()).unwrap() {
        Query::Series { countries, mode: SeriesMode::Keyed(CoverageScope::Table), .. } => assert_eq!((countries.len(), countries[1_499].key().as_ref()), (1_500, "T1499")),
        _ => panic!("a series of every member"),
    }
    let wide = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "territory", "measure": "controlledStates" }, "top": 1_000 } });
    assert!(matches!(crate::parse(&wide, &nobody()).unwrap(), Query::Series { mode: SeriesMode::Top { n: 1_000, least: false, .. }, .. }));
    let naval_top = serde_json::json!({ "product": "naval-losses", "params": { "branchId": "1", "countries": [], "top": 10, "attribution": "lost", "kind": "both", "period": "running" } });
    assert!(crate::parse(&naval_top, &nobody()).is_ok(), "running naval-losses top must parse");
    let naval_chosen = serde_json::json!({ "product": "naval-losses", "params": { "branchId": "1", "countries": ["GER", "JAP"], "top": null, "attribution": "sunkBy", "kind": "ships", "period": "running" } });
    assert!(crate::parse(&naval_chosen, &nobody()).is_ok(), "running naval-losses chosen countries must parse");
    let naval_month = serde_json::json!({ "product": "naval-losses", "params": { "branchId": "1", "countries": [], "top": 10, "attribution": "lost", "kind": "both", "period": "month" } });
    assert_eq!(
        crate::parse(&naval_month, &nobody()).err().unwrap().to_string(),
        "Monthly naval losses are not followed at the head"
    );
}

#[test]
fn closing_a_recording_drops_its_follows() {
    let (a, b) = (paper(), paper());
    let registry = crate::registry::Registry::new(Some(Arc::new(Papers(BTreeMap::from([("a".to_owned(), Arc::clone(&a)), ("b".to_owned(), Arc::clone(&b))])))));
    for (id, path) in [(1, "a"), (2, "a"), (3, "b")] {
        assert_eq!(registry.answer(std::path::Path::new(path), "follow", &follow_request(id, "country-measure", "politicalPower"), |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel"))).unwrap()["available"], true);
    }
    registry.close(std::path::Path::new("a"));
    assert_eq!(registry.followed(), 1);
    assert!(a.follow.declared().unwrap().is_empty(), "the closed recording's follows went");
    assert_eq!(b.follow.declared().unwrap().len(), 1, "the other recording's stayed");
}

fn one(id: u32, hour: Hour, value: i64) -> Vec<u8> {
    let mut message = Vec::new();
    crate::answer::begin(&mut message, 0, 0, hour, 0, false);
    crate::answer::push(&mut message, id, hour, &body(id, value));
    message
}

fn body(id: u32, value: i64) -> Vec<u8> {
    let mut out = Vec::new();
    let mut table = crate::answer::Table::default();
    table.clear(1, false);
    table.row(u128::from(id));
    table.cell(value, State::Value);
    table.encode(&mut out);
    out
}

#[test]
fn a_frame_round_trips_the_pipe() {
    use crate::push::{FOLLOW, read_frame, write_frame};
    let (mut reader, mut writer) = std::io::pipe().unwrap();
    let sent = one(7, 24, 123);
    write_frame(&mut writer, FOLLOW, 41, &sent).unwrap();
    let mut payload = Vec::new();
    assert_eq!(read_frame(&mut reader, &mut payload).unwrap(), Some((FOLLOW, 41)));
    assert_eq!(payload, sent, "byte for byte");
}

#[test]
fn a_message_past_a_frame_part_reaches_the_reader_whole() {
    use crate::push::{FOLLOW, Push, read_frame};
    let push = Push::default();
    let large: Vec<u8> = (0..(9u32 << 20) + 5).map(|at| (at % 251) as u8).collect();
    push.offer(large.clone());
    push.offer(one(7, 24, 123));
    push.stop();
    let mut pipe = Vec::new();
    push.drain(&mut pipe).expect("a message of any size is written");
    let mut input = pipe.as_slice();
    let mut payload = Vec::new();
    assert_eq!(read_frame(&mut input, &mut payload).unwrap(), Some((FOLLOW, 0)));
    assert!(payload == large, "the large message, joined from its parts: {} bytes of {}", payload.len(), large.len());
    assert_eq!(read_frame(&mut input, &mut payload).unwrap(), Some((FOLLOW, 1)));
    assert_eq!(payload, one(7, 24, 123), "and the message after it");
    assert_eq!(read_frame(&mut input, &mut payload).unwrap(), None);
}

#[test]
fn a_failed_write_stops_the_pushed_answers_by_name() {
    use crate::push::Push;
    struct Gone;
    impl std::io::Write for Gone {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "the reader is gone"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let push = Push::default();
    push.offer(one(7, 24, 1));
    push.offer(one(7, 48, 2));
    assert!(push.drain(&mut Gone).is_err());
    let failed = push.failed().expect("the failure, by name");
    assert!(failed.contains("the reader is gone"), "{failed}");
    assert_eq!((push.counters().written, push.counters().unwritten), (0, 2));
    push.offer(one(7, 72, 3));
    assert_eq!(push.counters().offered, 2, "nothing is queued once stopped");
}

#[test]
fn push_queues_charge_one_budget() {
    use crate::push::Push;
    let packet = one(7, 24, 1);
    let one_charge = crate::answer::charge(&packet);
    let budget = playhead::ByteBudget::new(one_charge * 4);
    let first = Push::with_budget(budget.clone());
    let second = Push::with_budget(budget.clone());
    first.offer(packet.clone());
    second.offer(packet);
    second.offer(one(7, 48, 2));
    assert_eq!(budget.used(), one_charge * 3);
    assert!(second.failed().is_none());
    first.take_waiting();
    drop(second);
    assert_eq!(budget.used(), 0);
}

#[test]
fn a_seek_sends_one_reset_message_and_follows_survive_it() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    harness.follow.follow(1, keys(&[100], &[0])).unwrap();
    harness.seek(0);
    harness.play(200);
    harness.seek(600);
    let played = harness.play(700);
    let resets: Vec<&Played> = played.iter().filter(|played| played.stats.reset).collect();
    assert_eq!(resets.len(), 1, "one reset message");
    assert!(resets[0].message.as_ref().is_some_and(|message| message.reset));
    let after: Vec<(Hour, Answer)> = played.iter().filter(|played| played.head >= 600).flat_map(Played::answers).map(|(_, hour, answer)| (hour, answer)).collect();
    assert!(after.len() > 3, "still followed after the seek");
    for (hour, answer) in after {
        assert_eq!(answer.values, vec![i64::from(hour) * 10], "hour {hour}");
    }
    assert_eq!(harness.follow.held().queries, 1);
}

#[test]
fn a_reset_sends_every_answer_again_whole_though_equal_to_the_last_sent() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    harness.follow.follow(1, keys(&[105], &[3])).unwrap();
    harness.follow.follow(2, Query::Cells { table: TABLE, columns: vec![3], rows: Rows::Live(vec![105]) }).unwrap();
    harness.seek(0);
    harness.play(48);
    harness.seek(24);
    let played = harness.play(24);
    let reset = played.iter().find(|played| played.stats.reset).expect("the seek's reset step");
    let answers = reset.answers();
    let mut ids: Vec<u32> = answers.iter().map(|(id, ..)| *id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2], "each answer again in the reset's step");
    for (id, hour, answer) in &answers {
        assert!(!answer.changed, "query {id}: whole, never rows changed over rows the reader may not hold");
        assert_eq!(held_rows(answer), held_rows(&harness.fresh(&keys(&[105], &[3]), *hour)), "query {id} at hour {hour}");
    }
}

fn golden() -> Vec<u8> {
    use crate::answer::{Table, begin, not_recorded, push};
    let mut message = Vec::new();
    begin(&mut message, 0, 0, 1234, 0, true);
    let mut table = Table::default();
    table.clear(1, false);
    table.row(100);
    table.name("GER");
    table.cell(12_345_678, State::Value);
    table.row(101);
    table.name("SOV");
    table.cell(0, State::Missing);
    table.explain("SOV has no value");
    table.row(0);
    table.name("ITA");
    table.absent("ITA is held by no country");
    let mut body = Vec::new();
    table.encode(&mut body);
    push(&mut message, 7, 1200, &body);
    body.clear();
    not_recorded("Countries was not recorded at this hour", &mut body);
    push(&mut message, 8, 1234, &body);
    table.clear(2, true);
    table.row((1u128 << 70) + 5);
    table.hours.push(1100);
    table.cell(-2, State::Value);
    table.cell(9, State::Value);
    body.clear();
    table.encode(&mut body);
    push(&mut message, 9, 1234, &body);
    message
}

#[test]
fn the_message_bytes_are_the_ones_the_ui_reads() {
    let hex: String = golden().iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(hex, GOLDEN);
}

const GOLDEN: &str = "464f4c3301000000d204000003000000000000000000000007000000b0040000d000000000000000010000000300000001000000050000006400000000000000650000000000000000000000000000000000000000000000000000000000000000000000000000004e61bc0000000000000000000000000000000000000000000001030000000000ffffffff02000000040000000000000000000000010000000300000000000000000000000300000006000000160000001900000032000000474552534f56534f5620686173206e6f2076616c75654954414954412069732068656c64206279206e6f20636f756e74727900000000000008000000d20400004000000000000000000000000000000000000000010000000000000027000000436f756e747269657320776173206e6f74207265636f72646564206174207468697320686f75720009000000d204000058000000000000000200000001000000020000000000000005000000000000004000000000000000feffffffffffffff09000000000000004c040000000000000000000000000000ffffffff00000000ffffffff000000000000000000000000";

#[test]
fn a_table_never_recorded_in_the_files_is_not_held() {
    let kinds = vec![KINDS.to_vec(); TABLE + 1];
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing { cap: 256, ..Writing::at(3) }).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..BLOCK + 10 {
        store.begin_hour(at as Hour, &[]).unwrap();
        for table in 0..TABLE {
            store.status(table, Status::Complete, true, Some(at as Hour), true).unwrap();
        }
        store.status(TABLE, Status::NotRecorded, false, None, true).unwrap();
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    store.end_run(&mut memory).unwrap();
    let pages = Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds };
    assert_eq!(crate::host::held_tables(&pages, TABLE + 1), vec![true, true, false], "a table whose every hour is not recorded is not the playhead's to follow");
    let recorded = record(quiet);
    assert!(crate::host::held_tables(&*recorded, TABLE + 1)[TABLE], "one not recorded for some hours is");
}

fn brief(at: u32, changes: &mut TableChanges<'_>) {
    quiet(at, changes);
    if at >= 1
        && born(at - 1)
        && let Some(slot) = changes.slot(1000 + u128::from(at - 1))
    {
        changes.delete(slot).unwrap();
    }
    if born(at) {
        let slot = changes.insert(1000 + u128::from(at)).unwrap();
        changes.write(slot, 0, Cell::I32(at as i32)).unwrap();
        changes.write(slot, 3, Cell::I64(-i64::from(at))).unwrap();
    }
}

#[test]
fn a_log_pushes_a_row_born_and_gone_inside_a_step_once() {
    let pages = record(brief);
    let mut harness = harness(&pages);
    harness.follow.follow(4, Query::Cells { table: TABLE, columns: vec![0, 3], rows: Rows::Appended }).unwrap();
    harness.seek(0);
    let played = harness.play(1000);
    let mut rows: Vec<(u128, Hour, i64, i64)> = Vec::new();
    for (_, hour, answer) in played.iter().flat_map(Played::answers) {
        let hours = answer.hours.clone().expect("rows appended");
        for (row, &born) in hours.iter().enumerate() {
            assert!(born <= hour);
            rows.push((answer.keys[row], born, answer.values[row * 2], answer.values[row * 2 + 1]));
        }
    }
    let last = played.last().expect("steps").head;
    let wanted: Vec<(u128, Hour, i64, i64)> = (1..=last).filter(|&at| born(at as u32)).map(|at| (1000 + at as u128, at, i64::from(at), -i64::from(at))).collect();
    assert!(wanted.len() > 50);
    assert_eq!(rows, wanted);
}

const LOG: [Kind; 4] = [Kind::I64, Kind::OptionalU32, Kind::I32, Kind::U16];

fn log_hour(at: u32, changes: &mut TableChanges<'_>) {
    let listed = |changes: &TableChanges<'_>| changes.held().listed().unwrap().ones().collect::<Vec<u64>>();
    if at % 7 == 0
        && let Some(&oldest) = listed(changes).first()
    {
        changes.forget(oldest).unwrap();
    }
    let count = at % 4 + u32::from(at % 13 == 0);
    for index in 0..count {
        let seed = u64::from(at) * 7 + u64::from(index);
        let cells = [
            Some(Cell::I64(seed as i64 * -1_000_003)),
            (seed % 4 != 0).then(|| Cell::OptionalU32((seed % 3 != 0).then_some(seed as u32 * 11))),
            Some(Cell::I32(at as i32)),
            Some(Cell::U16(index as u16)),
        ];
        let ordinal = changes.append(u128::from(at) << 64 | u128::from(index), &cells).unwrap();
        if at % 13 == 0 && index + 1 == count {
            changes.forget(ordinal).unwrap();
        }
    }
    if at % 50 == 0 {
        for &ordinal in listed(changes).iter().take(10).step_by(2) {
            changes.forget(ordinal).unwrap();
        }
    }
    let held = listed(changes);
    let excess = &held[..held.len().saturating_sub(30)];
    let mut start = 0;
    while start < excess.len() {
        let run = excess[start..].iter().enumerate().take_while(|&(offset, &ordinal)| ordinal == excess[start] + offset as u64).count();
        changes.forget_run(excess[start], run as u64).unwrap();
        start += run;
    }
}

fn record_log() -> Arc<Recording> {
    let kinds = vec![LOG.to_vec(); TABLE + 1];
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    let mut logs = vec![false; kinds.len()];
    logs[TABLE] = true;
    store.declare_logs(&logs).unwrap();
    store.attach(Encoder::new(&kinds, Writing { cap: 256, ..Writing::at(3) }).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..HOURS {
        store.begin_hour(at as Hour, &[]).unwrap();
        for table in 0..=TABLE {
            store.status(table, Status::Complete, true, Some(at as Hour), true).unwrap();
        }
        log_hour(at, &mut store.table(TABLE).unwrap());
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    Arc::new(Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds })
}

type LogRow = (u128, Hour, Vec<(i64, State)>);

fn log_read_directly(pages: &Recording, columns: &[usize]) -> (Vec<(u64, LogRow)>, Vec<(Hour, u64)>) {
    let run = &pages.runs[0];
    let table_pages = pages.table_pages(0, TABLE, 0, run.blocks()).unwrap();
    let (mut appended, mut forgotten) = (Vec::new(), Vec::new());
    replay_columnar::unit::reader::log_span(&pages.bytes, &table_pages, &LOG, TABLE, columns, run.first_hour, Hour::MIN..=Hour::MAX, &mut |hour, event| match event {
        replay_columnar::unit::reader::LogEvent::Appended { ordinal, key, rows, at, .. } => {
            appended.push((ordinal, (key, hour, columns.iter().map(|&column| crate::evaluate::number(rows.value(column, at))).collect())));
        }
        replay_columnar::unit::reader::LogEvent::Forgotten { from, to, .. } => forgotten.extend((from..to).map(|ordinal| (hour, ordinal))),
        replay_columnar::unit::reader::LogEvent::Named { .. } => {}
    })
    .unwrap();
    (appended, forgotten)
}

fn pushed(answer: &Answer) -> Vec<LogRow> {
    let hours = answer.hours.clone().expect("rows appended");
    (0..answer.keys.len()).map(|row| (answer.keys[row], hours[row], (0..answer.columns).map(|column| (answer.values[row * answer.columns + column], answer.states[row * answer.columns + column])).collect())).collect()
}

fn since_the_reset(mut played: Vec<Played>) -> Vec<Played> {
    let reset = played.iter().position(|played| played.stats.reset).expect("the seek's reset");
    played.split_off(reset)
}

#[test]
fn a_followed_feed_of_a_log_pushes_what_its_span_gives() {
    let pages = record_log();
    let columns = vec![0, 1, 2];
    let (appended, _) = log_read_directly(&pages, &columns);
    for every in [1, 24, 240] {
        let mut harness = stepping(&pages, every);
        harness.follow.follow(1, Query::Cells { table: TABLE, columns: columns.clone(), rows: Rows::Appended }).unwrap();
        assert_eq!(harness.follow.declared().unwrap(), vec![playhead::Declared { table: TABLE, columns: columns.clone(), keyed: true, passing: false, hourly: false }]);
        let mut stretches: Vec<(Hour, Hour, Vec<LogRow>)> = Vec::new();
        for (seek, to) in [(0, 400), (1000, 1250), (600, 900), (1300, HOURS as Hour - 1)] {
            harness.seek(seek);
            let mut stretch = (seek, seek, Vec::new());
            for played in since_the_reset(harness.play(to)) {
                stretch.1 = stretch.1.max(played.head);
                stretch.2.extend(played.answers().iter().flat_map(|(_, _, answer)| pushed(answer)));
            }
            stretches.push(stretch);
        }
        for (from, to, rows) in stretches {
            let wanted: Vec<LogRow> = appended.iter().filter(|(_, row)| from < row.1 && row.1 <= to).map(|(_, row)| row.clone()).collect();
            assert!(!wanted.is_empty(), "rows in {from}..{to}");
            assert_eq!(rows, wanted, "{every} hours a step: the stretch {from}..={to}");
        }
    }
}

#[test]
fn a_followed_list_of_a_log_holds_its_listed_rows_at_twenty_seeks() {
    let pages = record_log();
    let columns = vec![0, 1, 2];
    let (appended, forgotten) = log_read_directly(&pages, &columns);
    let rows: BTreeMap<u64, LogRow> = appended.iter().cloned().collect();
    let listed = |hour: Hour| -> BTreeMap<u64, &LogRow> {
        let mut listed: BTreeMap<u64, &LogRow> = rows.iter().filter(|(_, row)| row.1 <= hour).map(|(&ordinal, row)| (ordinal, row)).collect();
        for &(_, ordinal) in forgotten.iter().filter(|(at, _)| *at <= hour) {
            listed.remove(&ordinal);
        }
        listed
    };
    let listed_at = |hour: Hour| -> Vec<(u128, Vec<(i64, State)>)> { listed(hour).values().map(|row| (row.0, row.2.clone())).collect() };
    let mut harness = harness(&pages);
    harness.follow.follow(2, Query::Cells { table: TABLE, columns: columns.clone(), rows: Rows::Listed }).unwrap();
    let mut answered = 0;
    for seek in [0, 1, 37, 719, 720, 721, 1000, 1150, 100, 500, 1190, 13, 50, 650, 900, 1100, 1200, 300, 800, 1180] {
        harness.seek(seek);
        let mut played = since_the_reset(harness.play(seek + 24));
        assert_eq!(decode(&played[0].message.as_ref().unwrap().bytes).unwrap().hour, seek);
        let ordinals: Vec<u64> = listed(seek).keys().copied().collect();
        let read = harness.playhead().rows(Wanted { run: 0, table: TABLE, ordinals: ordinals.clone(), columns: columns.clone() }).wait();
        assert_eq!(read.map(|rows| rows.ordinals().to_vec()).map_err(|error| error.to_string()), Ok(ordinals), "the rows listed at {seek}, read by ordinal");
        played.extend(harness.play(seek + 24 + 6 * 24));
        let answers: Vec<(Hour, Answer)> = played.iter().flat_map(Played::answers).map(|(_, hour, answer)| (hour, answer)).collect();
        let steps: Vec<_> = played.iter().map(|played| (played.stats.first, played.stats.last, played.stats.reset, played.head, played.stats.touched, played.stats.evaluated, played.stats.changed, played.stats.sent)).collect();
        assert!(answers.iter().any(|(hour, _)| *hour >= seek), "an answer after the seek to {seek}: the steps (first, last, reset, head, touched, evaluated, changed, sent) {steps:?}; {:?}", harness.follow.counters());
        for (hour, answer) in answers {
            assert!(answer.not_recorded.is_none() && answer.hours.is_none(), "rows at {hour}");
            let held: Vec<(u128, Vec<(i64, State)>)> = (0..answer.keys.len()).map(|row| (answer.keys[row], (0..answer.columns).map(|column| (answer.values[row * answer.columns + column], answer.states[row * answer.columns + column])).collect())).collect();
            assert_eq!(held, listed_at(hour), "the rows listed at {hour}, after the seek to {seek}");
            answered += 1;
        }
    }
    assert!(answered >= 20);
}

#[test]
fn a_listed_log_added_at_the_final_paused_hour_delivers_when_its_read_completes() {
    let pages = record_log();
    let columns = vec![0, 1, 2];
    let (appended, forgotten) = log_read_directly(&pages, &columns);
    let rows: BTreeMap<u64, LogRow> = appended.iter().cloned().collect();
    let last = (HOURS - 1) as Hour;
    let mut expected: BTreeMap<u64, &LogRow> = rows.iter().filter(|(_, row)| row.1 <= last).map(|(&ordinal, row)| (ordinal, row)).collect();
    for &(_, ordinal) in forgotten.iter().filter(|(at, _)| *at <= last) {
        expected.remove(&ordinal);
    }
    let want: Vec<(u128, Vec<(i64, State)>)> = expected.values().map(|row| (row.0, row.2.clone())).collect();
    let mut harness = harness(&pages);
    harness.playhead().lead(playhead::Lead { seconds: 1.0, bytes: 8 << 20 });
    harness.seek(last);
    let _ = harness.play(last);
    harness.follow.follow(2, Query::Cells { table: TABLE, columns, rows: Rows::Listed }).unwrap();
    let answer = loop {
        let played = harness.played.recv().unwrap();
        if let Some((_, _, answer)) = played.answers().into_iter().find(|(id, _, _)| *id == 2) {
            break answer;
        }
    };
    assert!(answer.not_recorded.is_none() && answer.hours.is_none());
    let held: Vec<(u128, Vec<(i64, State)>)> = (0..answer.keys.len())
        .map(|row| (answer.keys[row], (0..answer.columns).map(|column| (answer.values[row * answer.columns + column], answer.states[row * answer.columns + column])).collect()))
        .collect();
    assert_eq!(held, want);
}

#[derive(Default)]
struct Gate {
    held: Mutex<(bool, usize, usize)>,
    moved: Condvar,
}

impl Gate {
    fn close(&self) {
        *self.held.lock().unwrap() = (true, 0, 0);
    }

    fn open(&self) {
        self.held.lock().unwrap().0 = false;
        self.moved.notify_all();
    }

    fn open_one(&self) {
        self.held.lock().unwrap().2 += 1;
        self.moved.notify_all();
    }

    fn wait_entered(&self, reads: usize) {
        let mut held = self.held.lock().unwrap();
        while held.1 < reads {
            held = self.moved.wait(held).unwrap();
        }
    }

    fn pass(&self) {
        let mut held = self.held.lock().unwrap();
        held.1 += 1;
        self.moved.notify_all();
        while held.0 && held.2 == 0 {
            held = self.moved.wait(held).unwrap();
        }
        if held.0 {
            held.2 -= 1;
        }
    }
}

struct Gated {
    pages: Arc<Recording>,
    gate: Arc<Gate>,
}

impl Pages for Gated {
    fn extents(&self, run: usize) -> &dyn Extents {
        self.pages.extents(run)
    }

    fn runs(&self) -> &[Run] {
        self.pages.runs()
    }

    fn kinds(&self, table: usize) -> Option<&[Kind]> {
        self.gate.pass();
        self.pages.kinds(table)
    }
}

struct HeldReads {
    gate: Arc<Gate>,
    rows: Playhead,
}

impl Drop for HeldReads {
    fn drop(&mut self) {
        self.gate.open();
    }
}

fn held_reads(pages: &Arc<Recording>) -> HeldReads {
    let gate = Arc::new(Gate::default());
    let gated: Arc<dyn Pages> = Arc::new(Gated { pages: Arc::clone(pages), gate: Arc::clone(&gate) });
    let rows = Playhead::new(Arc::clone(&gated), Timeline::whole(&*gated).unwrap(), Settings::default(), None).unwrap();
    HeldReads { gate, rows }
}

type ListedRows = Vec<(u128, Vec<(i64, State)>, Option<String>)>;

fn listed_at(pages: &Recording, columns: &[usize]) -> impl Fn(Hour) -> ListedRows + use<> {
    let (appended, forgotten) = log_read_directly(pages, columns);
    move |hour| {
        let mut listed: BTreeMap<u64, (u128, Vec<(i64, State)>, Option<String>)> =
            appended.iter().filter(|(_, row)| row.1 <= hour).map(|(ordinal, row)| (*ordinal, (row.0, row.2.clone(), None))).collect();
        for (_, ordinal) in forgotten.iter().filter(|(at, _)| *at <= hour) {
            listed.remove(ordinal);
        }
        listed.into_values().collect()
    }
}

fn limits(played: &Played) -> Vec<(u32, Hour)> {
    played.message.as_ref().map(|message| decode(&message.bytes).unwrap().limited).unwrap_or_default()
}

#[test]
fn a_listed_read_that_lands_after_read_ahead_answers_the_hour_it_was_asked_for() {
    const ASKED: Hour = 300;
    let pages = record_log();
    let (columns, fewer) = (vec![0, 1, 2], vec![0, 2]);
    let (listed, listed_fewer) = (listed_at(&pages, &columns), listed_at(&pages, &fewer));
    let reads = held_reads(&pages);
    let mut harness = stepping_with(&pages, 1, Timeline::whole(&*pages).unwrap(), |_| reads.rows.log_reads());
    reads.gate.close();
    harness.follow.follow(2, Query::Cells { table: TABLE, columns, rows: Rows::Listed }).unwrap();
    harness.seek(ASKED);
    let mut held = since_the_reset(harness.play(ASKED));
    reads.gate.wait_entered(1);
    while held.last().is_none_or(|played| played.stats.last <= ASKED) {
        held.push(harness.played.recv().unwrap());
    }
    harness.follow.follow(3, Query::Cells { table: TABLE, columns: fewer, rows: Rows::Listed }).unwrap();
    let second = loop {
        let played = harness.played.recv().unwrap();
        let activated = limits(&played).contains(&(3, Hour::MIN));
        let head = played.head;
        held.push(played);
        if activated {
            break head;
        }
    };
    assert!(held.iter().flat_map(Played::answers).all(|(id, ..)| id != 2 && id != 3), "no list answers while its rows are being read");
    assert!(held.iter().filter(|played| played.message.is_some()).all(|played| limits(played).contains(&(2, Hour::MIN))), "a list being read holds no hour");
    let (mut first, mut played): (BTreeMap<u32, (Hour, Answer, Vec<(u32, Hour)>, Hour)>, Vec<Played>) = (BTreeMap::new(), Vec::new());
    for (id, open) in [(2, Gate::open_one as fn(&Gate)), (3, Gate::open)] {
        open(&reads.gate);
        while !first.contains_key(&id) {
            let next = harness.played.recv().unwrap();
            for (id, hour, answer) in next.answers() {
                first.entry(id).or_insert_with(|| (hour, answer, limits(&next), next.stats.last));
            }
            played.push(next);
        }
    }
    let (hour, answer, pinned, sent) = &first[&2];
    assert!(*sent > ASKED, "read-ahead passed the asked hour before the rows were read");
    assert_eq!((*hour, held_rows(answer)), (ASKED, listed(ASKED)), "the rows read answer the hour they were asked for");
    assert!(pinned.contains(&(2, ASKED)), "the answer holds only its own hour: {pinned:?}");
    let (hour, answer, ..) = &first[&3];
    assert_eq!((*hour, held_rows(answer)), (second, listed_fewer(second)), "a second list answers the hour it was asked at");
    let sent = *sent;
    played.extend(harness.play(sent + 6));
    let answered = |played: &Played| played.answers().into_iter().find(|(id, ..)| *id == 2);
    let late = played.iter().position(|played| answered(played).is_some()).expect("the late answer");
    let next = played[late + 1..].iter().find(|played| answered(played).is_some() || played.stats.last > sent).expect("a step past the late answer");
    for between in played[late + 1..].iter().take_while(|played| !std::ptr::eq(*played, next)) {
        assert!(limits(between).contains(&(2, ASKED)), "every message before the list's own hour holds it at the hour read: {:?}", limits(between));
    }
    let Some((_, hour, answer)) = answered(next) else { panic!("the next step answers the list at its own hour: {:?}", next.answers()) };
    assert!(next.stats.last > sent, "the list's own hour is answered in a message past the late one");
    assert_eq!((hour, held_rows(&answer)), (next.head, listed(next.head)));
    for (id, hour, answer) in played.iter().flat_map(Played::answers) {
        let want = if id == 2 { listed(hour) } else { listed_fewer(hour) };
        assert_eq!(held_rows(&answer), want, "list {id} at {hour}");
    }
}

#[test]
fn a_list_read_again_at_a_new_piece_after_a_late_answer_answers_that_piece() {
    const ASKED: Hour = 300;
    const ENTERED: Hour = 305;
    let pages = record_log();
    let columns = vec![0, 1, 2];
    let listed = listed_at(&pages, &columns);
    let reads = held_reads(&pages);
    let pieces = vec![playhead::Piece { run: 0, first: 0, last: ENTERED - 2 }, playhead::Piece { run: 0, first: ENTERED, last: HOURS as Hour - 1 }];
    let mut harness = stepping_with(&pages, 1, Timeline::new(&*pages, pieces).unwrap(), |_| reads.rows.log_reads());
    reads.gate.close();
    harness.follow.follow(2, Query::Cells { table: TABLE, columns, rows: Rows::Listed }).unwrap();
    harness.seek(ASKED);
    let mut played = since_the_reset(harness.play(ASKED));
    reads.gate.wait_entered(1);
    while played.last().is_none_or(|played| played.stats.last < ENTERED - 2) {
        played.push(harness.played.recv().unwrap());
    }
    reads.gate.open_one();
    let late = loop {
        let next = harness.played.recv().unwrap();
        if let Some((_, hour, _)) = next.answers().into_iter().find(|(id, ..)| *id == 2) {
            break (hour, next.stats.last);
        }
    };
    assert_eq!(late, (ASKED, ENTERED - 2), "the first piece's last hour carries the late answer");
    played = harness.play(ENTERED + 2);
    assert!(played.iter().any(|played| played.stats.gap), "the gap between the pieces");
    reads.gate.wait_entered(2);
    assert!(played.iter().flat_map(Played::answers).all(|(id, hour, _)| id != 2 || hour < ENTERED), "the new piece's list waits for its rows");
    reads.gate.open();
    let next = harness.played.recv().unwrap();
    let answers = next.answers();
    let Some((_, hour, answer)) = answers.iter().find(|(id, ..)| *id == 2) else { panic!("the rows read at the new piece answer it: {answers:?}") };
    assert_eq!((*hour, held_rows(answer)), (ENTERED, listed(ENTERED)));
}

#[test]
fn a_listed_read_held_across_a_seek_or_an_unfollow_answers_only_what_is_asked_now() {
    const FIRST: Hour = 300;
    const SOUGHT: Hour = 900;
    let pages = record_log();
    let columns = vec![0, 1, 2];
    let listed = listed_at(&pages, &columns);
    let reads = held_reads(&pages);
    let mut harness = stepping_with(&pages, 1, Timeline::whole(&*pages).unwrap(), |_| reads.rows.log_reads());
    reads.gate.close();
    for id in [2, 3] {
        harness.follow.follow(id, Query::Cells { table: TABLE, columns: columns.clone(), rows: Rows::Listed }).unwrap();
    }
    harness.seek(FIRST);
    let mut played = harness.play(FIRST);
    reads.gate.wait_entered(1);
    harness.follow.unfollow(3).unwrap();
    harness.seek(SOUGHT);
    played.extend(harness.play(SOUGHT + 3));
    assert!(played.iter().flat_map(Played::answers).all(|(id, ..)| id != 2 && id != 3), "no list answers while its rows are being read");
    reads.gate.open();
    let (hour, answer) = loop {
        let played = harness.played.recv().unwrap();
        let answers = played.answers();
        assert!(answers.iter().all(|(id, ..)| *id != 3), "an unfollowed list is never answered");
        if let Some((_, hour, answer)) = answers.into_iter().find(|(id, ..)| *id == 2) {
            break (hour, answer);
        }
    };
    assert_eq!((hour, held_rows(&answer)), (SOUGHT, listed(SOUGHT)), "the rows asked before the seek answer nothing");
}

#[test]
fn a_followed_list_of_a_table_holding_state_is_refused_by_name() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    harness.follow.follow(3, Query::Cells { table: TABLE, columns: vec![0], rows: Rows::Listed }).unwrap();
    harness.seek(0);
    let refused = harness.play(48).iter().flat_map(Played::answers).find_map(|(_, _, answer)| answer.not_recorded).expect("an answer");
    assert!(refused.contains("Countries holds state"), "{refused}");
}

fn held_rows(answer: &Answer) -> Vec<(u128, Vec<(i64, State)>, Option<String>)> {
    (0..answer.rows())
        .map(|row| {
            let cells = (0..answer.columns).map(|column| (answer.values[row * answer.columns + column], answer.states[row * answer.columns + column])).collect();
            (answer.keys[row], cells, answer.reason(row).map(str::to_owned))
        })
        .collect()
}

fn lay(held: &mut [(u128, Vec<(i64, State)>, Option<String>)], changed: &Answer) {
    for row in held_rows(changed) {
        let at = held.iter().position(|(key, ..)| *key == row.0).expect("rows changed name only keys held");
        held[at] = row;
    }
}

#[test]
fn a_live_query_pushes_only_the_rows_that_changed_and_its_reader_holds_the_answer_at_every_step() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    let asked = [100, 105, 107, 999];
    harness.follow.follow(1, Query::Cells { table: TABLE, columns: vec![0, 3], rows: Rows::Live(asked.to_vec()) }).unwrap();
    assert!(harness.follow.follow(2, Query::Cells { table: TABLE, columns: vec![0], rows: Rows::Live(vec![105, 100]) }).is_err(), "live keys are named ascending");
    harness.seek(0);
    let played = harness.play(420);
    let mut held: Option<Vec<(u128, Vec<(i64, State)>, Option<String>)>> = None;
    let (mut whole, mut changes, mut unrecorded) = (0, 0, 0);
    for played in &played {
        for (_, hour, answer) in played.answers() {
            if let Some(reason) = &answer.not_recorded {
                assert_eq!(reason, "Countries not recorded at this hour", "no verb to agree with a plural table");
                held = None;
                unrecorded += 1;
                continue;
            }
            if answer.changed {
                let moved = moves_slot_5(played);
                assert_eq!(answer.keys, if moved { vec![100, 105] } else { vec![100] }, "only the rows that changed, hours {}..{}", played.from, played.head);
                lay(held.as_mut().expect("rows changed come only over rows held"), &answer);
                changes += 1;
            } else {
                assert_eq!(answer.keys, asked, "a whole answer holds every key asked");
                held = Some(held_rows(&answer));
                whole += 1;
            }
            let fresh = harness.fresh(&keys(&asked, &[0, 3]), hour);
            assert_eq!(held.as_deref(), Some(held_rows(&fresh).as_slice()), "the reader holds the answer at hour {hour}");
            assert_eq!(held.as_ref().unwrap()[3].2.as_deref(), Some(crate::live::NO_ROW), "a key with no row is missing by name, never zero");
        }
    }
    assert_eq!((whole, unrecorded), (2, 1), "whole at the first answer and again after the hours not recorded, which are said once");
    assert!(changes > 10, "{changes} answers of rows changed");
}

fn rows_message(hour: Hour, changed: bool, rows: &[(u128, i64)]) -> Vec<u8> {
    let mut table = crate::answer::Table::default();
    table.clear(1, false);
    for &(key, value) in rows {
        table.row(key);
        table.cell(value, State::Value);
    }
    let mut body = Vec::new();
    if changed { table.encode_changed(&mut body) } else { table.encode(&mut body) }
    let mut message = Vec::new();
    crate::answer::begin(&mut message, 0, 0, hour, 0, false);
    crate::answer::push(&mut message, 4, hour, &body);
    message
}

#[test]
fn a_live_step_reads_the_cells_of_the_rows_it_touched_only() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    harness.follow.follow(1, Query::Cells { table: TABLE, columns: vec![0, 2, 3], rows: Rows::Live(vec![100, 105, 106, 107]) }).unwrap();
    harness.seek(0);
    let played = harness.play(290);
    let first = played.iter().position(|played| !played.answers().is_empty()).expect("a first answer");
    assert_eq!(played[first].stats.cells, 4 * 3, "the first step reads every row once");
    let (mut checked, mut after_gap) = (0, false);
    for played in played[first + 1..].iter().filter(|played| played.moved()) {
        if unread(played.head as u32) {
            assert_eq!(played.stats.cells, 0, "hours {}..{}: Countries not recorded at the step's end", played.from, played.head);
            assert!(played.answers().iter().all(|(_, _, answer)| answer.not_recorded.is_some()), "hours {}..{}: only why", played.from, played.head);
            after_gap = true;
            continue;
        }
        if std::mem::take(&mut after_gap) {
            assert_eq!(played.stats.cells, 4 * 3, "hours {}..{}: every row read again after not recorded", played.from, played.head);
            continue;
        }
        let touched = 2 + u32::from(moves_slot_5(played));
        assert_eq!(played.stats.cells, 3 * touched, "hours {}..{}: the touched rows' cells, not the followed rows'", played.from, played.head);
        let changed: Vec<u128> = played.answers().iter().flat_map(|(_, _, answer)| answer.keys.clone()).collect();
        assert!(!changed.contains(&107) && (touched == 3 || !changed.contains(&105)), "only rows the step touched are sent: {changed:?}");
        checked += 1;
    }
    assert!(checked > 5, "{checked} steps checked");
}

#[test]
fn a_panel_past_the_budget_is_sent_whole_and_never_refused() {
    use crate::panel::dated;
    use crate::push::{PANEL, Panels, Push};
    let mut batch = dated::ProductBatch::new((0, 0, 0));
    batch.changed(1, 40, dated::DATA, b"small-one");
    batch.changed(2, 40, dated::DATA, &vec![b'x'; 2048]);
    batch.changed(3, 40, dated::DATA, b"small-three");
    let packet = batch.finish(Some(40));
    let budget = playhead::ByteBudget::new(2048);
    let push = Push::with_budget(budget.clone());
    let session = push.session();
    push.offer_panel(session, packet, Panels::All, false);
    assert!(push.failed().is_none());
    assert!(budget.full(), "the queue is charged past the budget, which holds read-ahead back");
    let (kind, sent, ..) = push.try_take().unwrap();
    assert_eq!(kind, PANEL);
    let records = dated::records(&sent).unwrap();
    assert_eq!(records.len(), 4);
    assert_eq!((records[0].id, records[0].kind, records[0].body), (1, dated::DATA, &b"small-one"[..]));
    assert_eq!((records[1].id, records[1].kind, records[1].body.len()), (2, dated::DATA, 2048), "capacity never turns a reading into the recording's refusal");
    assert_eq!((records[2].id, records[2].kind, records[2].body), (3, dated::DATA, &b"small-three"[..]));
    assert_eq!((records[3].id, records[3].kind, records[3].hour), (0, dated::COMPLETED, 40));
    let mut next = dated::ProductBatch::new((0, 0, 0));
    next.changed(1, 50, dated::DATA, b"still-alive");
    push.offer_panel(session, next.finish(Some(50)), Panels::All, false);
    assert!(push.failed().is_none());
    assert!(push.try_take().is_none(), "the next panel the clock has not reached waits for the page's credit");
    push.credit(session, crate::answer::charge(&sent) as u64);
    assert!(push.try_take().is_some(), "and leaves once the page releases the first");
}

#[test]
fn completed_product_step_has_constant_size_when_every_product_stands() {
    use crate::panel::dated;
    let next = dated::ProductBatch::new((0, 3, 9)).finish(Some(34));
    let rows = dated::records(&next).unwrap();
    assert_eq!(rows.iter().map(|r| (r.id, r.kind, r.hour)).collect::<Vec<_>>(), [(0, dated::COMPLETED, 34)]);
}

#[test]
fn an_answer_says_when_its_rows_ascend_by_key_and_reads_any_text_by_its_offset() {
    let mut table = crate::answer::Table::default();
    table.clear(1, false);
    for (key, name) in [(3, "Third"), (9, "Ninth"), (1u128 << 100, "Wide")] {
        table.row(key);
        table.cell(1, State::Value);
        table.name(name);
    }
    table.explain("This country was not recorded at this hour");
    let mut body = Vec::new();
    table.encode(&mut body);
    let answer = Answer::parse(&body).unwrap();
    assert!(answer.canonical, "keys ascend: the reader may merge rows by walking keys");
    assert_eq!((answer.name(1), answer.reason(2), answer.name(2)), (Some("Ninth"), Some("This country was not recorded at this hour"), Some("Wide")));
    table.clear(1, false);
    for key in [9, 3] {
        table.row(key);
        table.cell(1, State::Value);
    }
    body.clear();
    table.encode(&mut body);
    assert!(!Answer::parse(&body).unwrap().canonical, "rows in another order say so");
}

#[test]
fn a_send_waits_for_its_credit_but_one_message_always_goes_when_nothing_is_in_flight() {
    use crate::push::Push;
    let packet = one(7, 24, 1);
    let cost = crate::answer::charge(&packet);
    let budget = playhead::ByteBudget::new(cost * 4);
    let push = Push::with_budget(budget.clone());
    push.offer(packet);
    push.offer(one(7, 48, 2));
    push.offer(one(7, 72, 3));
    assert!(push.try_take().is_some());
    assert!(push.try_take().is_some(), "two packets fit the credit of half the budget");
    assert!(push.try_take().is_none(), "the third waits until the lanes release bytes");
    assert_eq!((push.in_flight(), budget.used()), (cost * 2, cost * 3), "credit is reserved from send until release");
    push.credit(0, cost as u64);
    assert!(push.try_take().is_some());
    push.credit(0, cost as u64 * 3);
    assert_eq!(budget.used(), 0);
    push.credit(0, cost as u64);
    assert_eq!(push.in_flight(), 0, "a retried release of a total already counted releases nothing again");
    let large: Vec<u8> = vec![0; cost * 3];
    push.offer(large);
    assert!(push.try_take().is_some(), "a message larger than the credit still goes alone");
}

#[test]
fn a_replacement_the_clock_has_reached_goes_while_the_lanes_still_hold_the_picture_it_replaces() {
    use crate::push::Push;
    let packet = one(7, 10, 1);
    let cost = crate::answer::charge(&packet);
    let budget = playhead::ByteBudget::new(cost * 5);
    let push = Push::with_budget(budget.clone());
    push.offer(packet);
    push.offer(one(7, 20, 2));
    push.offer(one(7, 30, 3));
    assert!(push.try_take().is_some() && push.try_take().is_some());
    assert!(push.try_take().is_none(), "ahead of the clock, a message waits for credit");
    push.head(0, 0, 30);
    assert!(push.try_take().is_some(), "the lanes keep their current picture until its replacement arrives: a reached hour always goes");
    let session = push.restart();
    assert_eq!((push.in_flight(), budget.used()), (0, 0), "a closed page's held bytes are forgotten once, by the one queue that charged them");
    push.offer(one(7, 40, 4));
    let (.., sent) = push.take_sent().unwrap();
    assert_eq!(sent, session, "a message names the session it was sent to");
    push.credit(session - 1, cost as u64);
    assert_eq!(push.in_flight(), cost, "an old page's late refund releases nothing of the new page's");
    push.credit(session, cost as u64);
    assert_eq!(push.in_flight(), 0);
    push.offer(one(7, 50, 5));
    assert!(push.try_take().is_some());
    push.credit(session, cost as u64);
    assert_eq!(push.in_flight(), cost, "review 6 A: the release total is idempotent, so a retry whose acknowledgement was lost frees nothing twice");
    push.offer(one(7, 60, 6));
    let retired = push.restart();
    assert!(push.try_take().is_none(), "review 6 A: a record queued for the old page is retired with its session, never sent as the new page's");
    assert_eq!((push.in_flight(), budget.used(), push.session()), (0, 0, retired));
}

fn routed(route: u32, hour: Hour, value: i64, held: u32) -> Vec<u8> {
    let mut message = Vec::new();
    crate::answer::begin(&mut message, route, 0, hour, 0, false);
    crate::answer::push_held(&mut message, 7, hour, &body(7, value), held);
    message
}

#[test]
fn a_changed_answer_reserves_the_table_it_lands_on_when_queued() {
    use crate::push::Push;
    let small = routed(0, 5, 1, 0);
    let budget = playhead::ByteBudget::new(small.len() * 10);
    let push = Push::with_budget(budget.clone());
    push.offer(small.clone());
    assert!(push.try_take().is_some());
    push.head(0, 0, 100);
    push.offer(routed(0, 10, 2, (small.len() * 12) as u32));
    assert!(push.failed().is_none() && budget.full(), "a table past the budget is queued and charged, never refused");
    assert!(push.try_take().is_some());
    let push = Push::with_budget(budget);
    push.offer(routed(0, 10, 2, (small.len() * 4) as u32));
    assert_eq!(push.in_flight(), 0);
    assert!(push.try_take().is_some());
    assert_eq!(push.in_flight(), 24 + 16 + 2 * small.len() * 4, "what was reserved while queued is what is held in flight");
}

#[test]
fn a_reached_update_goes_past_a_future_one_of_another_route_but_never_past_its_own() {
    use crate::{panel::dated, push::{FOLLOW, PANEL, Panels, Push}};
    let small = routed(1, 5, 1, 0);
    let small_len = small.len();
    let push = Push::with_budget(playhead::ByteBudget::new(small_len * 30));
    push.offer(small);
    assert!(push.try_take().is_some());
    push.offer(routed(1, 50, 2, (small_len * 8) as u32));
    assert!(push.try_take().is_none(), "the future update of route 1 waits for credit");
    push.offer(routed(2, 10, 3, 0));
    push.head(2, 0, 10);
    let (_, sent, ..) = push.try_take().expect("route 2's reached update goes past it");
    assert_eq!(crate::answer::decode(&sent).unwrap().route, 2);
    push.offer(routed(1, 11, 4, 0));
    push.head(1, 0, 11);
    assert!(push.try_take().is_none(), "route 1's later update never overtakes route 1's waiting one");

    let record = |id, hour, kind, body: &[u8]| dated::encoded(0, 3, 0, id, hour, kind, body);
    let sent = |bytes: &[u8]| dated::records(bytes).unwrap().into_iter().map(|r| (r.id, r.hour, r.kind)).collect::<Vec<_>>();
    let reset_follow = |hour, value| {
        let mut message = Vec::new();
        crate::answer::begin(&mut message, 3, 0, hour, 0, true);
        crate::answer::push_held(&mut message, 7, hour, &body(7, value), 0);
        message
    };

    let shown = record(39, 5, dated::DATA, &[1; 1000]);
    let one = crate::answer::charge(&shown);
    let push = Push::with_budget(playhead::ByteBudget::new(one * 3));
    let session = push.session();
    let mut drained = 0u64;
    push.head(3, 0, 10);
    push.offer_panel(session, shown, Panels::One(39), false);
    let (_, first, ..) = push.try_take().expect("a picture the page keeps");
    drained += crate::answer::charge(&first) as u64;
    push.offer_panel(session, record(40, 60, dated::DATA, &[2; 1000]), Panels::One(40), true);
    assert!(push.try_take().is_none(), "panel 40's lead record waits for credit");
    push.offer_panel(session, record(41, 10, dated::DATA, &[3; 8]), Panels::One(41), true);
    let (_, bytes, ..) = push.try_take().expect("panel 41's reached record goes past panel 40's lead record on the same route, even when both register live cells");
    drained += crate::answer::charge(&bytes) as u64;
    assert_eq!(sent(&bytes), [(41, 10, dated::DATA)]);
    push.offer_panel(session, dated::ProductBatch::new((0, 3, 0)).finish(Some(10)), Panels::All, false);
    assert!(push.try_take().is_none(), "a standalone route completion never overtakes a waiting record of any panel on its route");
    push.offer_panel(session, record(40, 10, dated::DATA, &[4; 8]), Panels::One(40), false);
    assert!(push.try_take().is_none(), "panel 40's later record never overtakes panel 40's waiting one");
    push.credit(session, drained);
    let mut order = Vec::new();
    while let Some((_, bytes, ..)) = push.try_take() {
        drained += crate::answer::charge(&bytes) as u64;
        order.extend(sent(&bytes));
        push.credit(session, drained);
    }
    assert_eq!(order, [(40, 60, dated::DATA), (0, 10, dated::COMPLETED), (40, 10, dated::DATA)], "released, the blocked line leaves in its own order");
    assert_eq!(push.in_flight(), 0, "crediting the exact sum of drained charges leaves nothing in flight");

    let push = Push::with_budget(playhead::ByteBudget::new(one * 2));
    let session = push.session();
    push.head(3, 0, 10);
    push.offer_panel(session, record(39, 5, dated::DATA, &[1; 1000]), Panels::One(39), false);
    assert!(push.try_take().is_some(), "a picture the page keeps, never released");
    push.offer_panel(session, record(42, 10, dated::DATA, &[5; 1000]), Panels::One(42), false);
    push.offer_panel(session, record(42, 60, dated::STANDS, &[]), Panels::One(42), false);
    push.offer_panel(session, record(43, 10, dated::DATA, &[6; 8]), Panels::One(43), false);
    let (_, bytes, ..) = push.try_take().expect("case 1: panel 42's reached data leaves past credit");
    assert_eq!(sent(&bytes), [(42, 10, dated::DATA)]);

    let push = Push::with_budget(playhead::ByteBudget::new(one * 2));
    let session = push.session();
    push.head(3, 0, 10);
    push.offer_panel(session, record(39, 5, dated::DATA, &[1; 1000]), Panels::One(39), false);
    assert!(push.try_take().is_some());
    let mut completing = dated::ProductBatch::new((0, 3, 0));
    completing.changed(42, 10, dated::DATA, &[5; 1000]);
    push.offer_panel(session, completing.finish(Some(10)), Panels::All, false);
    push.offer_panel(session, record(42, 60, dated::STANDS, &[]), Panels::One(42), false);
    push.offer_panel(session, record(43, 10, dated::DATA, &[6; 8]), Panels::One(43), false);
    let (_, bytes, ..) = push.try_take().expect("case 2: the reached completion leaves first");
    assert_eq!(sent(&bytes), [(42, 10, dated::DATA), (0, 10, dated::COMPLETED)]);

    let push = Push::with_budget(playhead::ByteBudget::new(1 << 20));
    let session = push.session();
    push.head(3, 0, 10);
    push.offer_follow(session, reset_follow(10, 1));
    push.offer_panel(session, record(45, 10, dated::DATA, &[8; 8]), Panels::One(45), false);
    push.offer_follow(session, routed(3, 10, 2, 0));
    let kinds: Vec<u8> = std::iter::from_fn(|| push.try_take()).map(|(kind, ..)| kind).collect();
    assert_eq!(kinds, [FOLLOW, PANEL, FOLLOW]);

    let push = Push::with_budget(playhead::ByteBudget::new(1 << 20));
    let session = push.session();
    push.head(3, 0, 10);
    push.offer_follow(session, routed(3, 10, 1, 0));
    push.offer_panel(session, record(45, 10, dated::DATA, &[8; 8]), Panels::One(45), false);
    push.offer_follow(session, reset_follow(10, 2));
    let kinds: Vec<u8> = std::iter::from_fn(|| push.try_take()).map(|(kind, ..)| kind).collect();
    assert_eq!(kinds, [FOLLOW, PANEL, FOLLOW]);

    let push = Push::with_budget(playhead::ByteBudget::new(1 << 20));
    let session = push.session();
    push.head(3, 0, 20);
    push.offer_panel(session, record(46, 10, dated::DATA, &[9; 8]), Panels::One(46), true);
    let mut cell = Vec::new();
    crate::answer::begin(&mut cell, 3, 0, 20, 0, false);
    crate::answer::push_held(&mut cell, crate::live::CELLS | 5, 20, &body(7, 3), 0);
    push.offer_follow(session, cell);
    push.offer_panel(session, dated::ProductBatch::new((0, 3, 0)).finish(Some(20)), Panels::All, false);
    let kinds: Vec<u8> = std::iter::from_fn(|| push.try_take()).map(|(kind, ..)| kind).collect();
    assert_eq!(kinds, [PANEL, FOLLOW, PANEL]);

    let push = Push::with_budget(playhead::ByteBudget::new(1 << 20));
    let session = push.session();
    push.head(3, 0, 20);
    push.offer_panel(session, record(47, 10, dated::DATA, &[10; 8]), Panels::One(47), true);
    push.offer_follow(session, routed(3, 20, 4, 0));
    push.offer_panel(session, dated::ProductBatch::new((0, 3, 0)).finish(Some(20)), Panels::All, false);
    let kinds: Vec<u8> = std::iter::from_fn(|| push.try_take()).map(|(kind, ..)| kind).collect();
    assert_eq!(kinds, [PANEL, FOLLOW, PANEL]);

    let hold = record(50, 5, dated::DATA, &[1; 1500]);
    let small_b = record(52, 12, dated::DATA, &[2; 8]);
    let hold_charge = crate::answer::charge(&hold) as u64;
    let b_charge = crate::answer::charge(&small_b) as u64;
    let push = Push::with_budget(playhead::ByteBudget::new(6350));
    let session = push.session();
    push.head(3, 0, 10);
    push.offer_panel(session, hold.clone(), Panels::One(50), false);
    assert!(push.try_take().is_some());
    push.offer_panel(session, record(51, 11, dated::DATA, &[3; 1000]), Panels::One(51), false);
    push.offer_panel(session, small_b.clone(), Panels::One(52), false);
    push.offer_follow(session, routed(3, 60, 1, 500));
    push.offer_panel(session, record(53, 11, dated::DATA, &[4; 8]), Panels::One(53), false);
    let (_, bytes, ..) = push.try_take().expect("case S1: small B leaves at hour 10 while large A waits for credit");
    assert_eq!(sent(&bytes), [(52, 12, dated::DATA)]);
    assert!(push.try_take().is_none());
    push.head(3, 0, 11);
    let (_, bytes, ..) = push.try_take().expect("case S1a: at hour 11 cell-free A leaves first");
    assert_eq!(sent(&bytes), [(51, 11, dated::DATA)]);
    let (_, bytes, ..) = push.try_take().expect("case S1a: cell-free C then passes future FOLLOW(60)");
    assert_eq!(sent(&bytes), [(53, 11, dated::DATA)]);
    assert!(push.try_take().is_none(), "case S1a: future FOLLOW(60) still waits for credit");
    push.credit(session, hold_charge + b_charge);
    let (kind, ..) = push.try_take().expect("case S1a: crediting hold and B releases future FOLLOW(60)");
    assert_eq!(kind, FOLLOW);
    assert!(push.try_take().is_none());

    let push = Push::with_budget(playhead::ByteBudget::new(6350));
    let session = push.session();
    push.head(3, 0, 10);
    push.offer_panel(session, hold, Panels::One(50), false);
    assert!(push.try_take().is_some());
    push.offer_panel(session, record(51, 11, dated::DATA, &[3; 1000]), Panels::One(51), false);
    push.offer_panel(session, small_b, Panels::One(52), false);
    push.offer_follow(session, routed(3, 60, 1, 500));
    push.offer_panel(session, record(53, 11, dated::DATA, &[4; 8]), Panels::One(53), true);
    let (_, bytes, ..) = push.try_take().expect("case S1b: B leaves at hour 10");
    assert_eq!(sent(&bytes), [(52, 12, dated::DATA)]);
    assert!(push.try_take().is_none());
    push.head(3, 0, 11);
    let (_, bytes, ..) = push.try_take().expect("case S1b: A leaves immediately at hour 11");
    assert_eq!(sent(&bytes), [(51, 11, dated::DATA)]);
    assert!(push.try_take().is_none(), "case S1b: cell-registering C waits behind future FOLLOW(60)");
    push.credit(session, hold_charge);
    let (kind, ..) = push.try_take().expect("case S1b: crediting hold releases future FOLLOW(60) ahead of C");
    assert_eq!(kind, FOLLOW);
    let (kind, bytes, ..) = push.try_take().expect("case S1b: cell-registering C leaves behind FOLLOW(60)");
    assert_eq!(kind, PANEL);
    assert_eq!(sent(&bytes), [(53, 11, dated::DATA)]);
    assert!(push.try_take().is_none());

    let push = Push::with_budget(playhead::ByteBudget::new(one * 3));
    let session = push.session();
    push.head(3, 0, 10);
    push.offer_panel(session, record(54, 5, dated::DATA, &[1; 1000]), Panels::One(54), false);
    assert!(push.try_take().is_some());
    push.offer_panel(session, record(55, 60, dated::DATA, &[2; 1000]), Panels::One(55), true);
    let mut confirm = Vec::new();
    crate::answer::begin(&mut confirm, 3, 0, 10, 0, false);
    push.offer_follow(session, confirm);
    assert!(push.try_take().is_none(), "case S1b: reached FOL3 confirmation never overtakes a queued declaring panel");

    for use_completed in [true, false] {
        let filler = record(60, 5, dated::DATA, &[1; 2000]);
        let fill_charge = crate::answer::charge(&filler);
        let push = Push::with_budget(playhead::ByteBudget::new(fill_charge));
        let session = push.session();
        push.head(3, 0, 10);
        push.offer_panel(session, filler, Panels::One(60), false);
        assert!(push.try_take().is_some(), "budget-filling packet in flight");
        push.offer_panel(session, record(61, 11, dated::DATA, &[2; 1000]), Panels::One(61), false);
        push.offer_panel(session, record(62, 10, dated::DATA, &[3; 8]), Panels::One(62), false);
        let (_, bytes, ..) = push.try_take().expect("case S2: B(10) leaves under the reached exception while A(11) stays queued");
        assert_eq!(sent(&bytes), [(62, 10, dated::DATA)]);
        if use_completed {
            push.offer_panel(session, dated::ProductBatch::new((0, 3, 0)).finish(Some(12)), Panels::All, false);
        } else {
            push.offer_panel(session, record(61, 12, dated::DATA, &[4; 8]), Panels::One(61), false);
        }
        push.offer_panel(session, record(63, 11, dated::DATA, &[5; 8]), Panels::One(63), false);
        push.head(3, 0, 11);
        push.credit(session, fill_charge as u64);
        let (_, bytes, ..) = push.try_take().expect("case S2: A(11) leaves before COMPLETED(12) or A(12)");
        assert_eq!(sent(&bytes), [(61, 11, dated::DATA)]);
    }

    let push = Push::with_budget(playhead::ByteBudget::new(one * 3));
    let session = push.session();
    push.head(3, 0, 10);
    push.offer_panel(session, record(70, 5, dated::DATA, &[1; 1000]), Panels::One(70), false);
    assert!(push.try_take().is_some());
    push.offer_panel(session, record(71, 60, dated::DATA, &[2; 1000]), Panels::One(71), false);
    push.offer_panel(session, record(72, 10, dated::DATA, &[3; 8]), Panels::One(72), false);
    push.offer_panel(session, dated::ProductBatch::new((0, 3, 0)).finish(Some(10)), Panels::All, false);
    let (_, bytes, ..) = push.try_take().expect("panel 72 still overtakes panel 71");
    assert_eq!(sent(&bytes), [(72, 10, dated::DATA)]);
    assert!(push.try_take().is_none(), "panel 71 and the completion wait for credit");

    let filler = record(80, 5, dated::DATA, &[1; 5000]);
    let filler_charge = crate::answer::charge(&filler);
    let completion = dated::ProductBatch::new((0, 3, 0)).finish(Some(11));
    let completion_charge = crate::answer::charge(&completion);
    let push = Push::with_budget(playhead::ByteBudget::new(filler_charge * 2));
    let session = push.session();
    push.head(3, 0, 10);
    push.offer_panel(session, filler, Panels::One(80), false);
    assert!(push.try_take().is_some());
    push.offer_follow(session, routed(3, 11, 1, 2000));
    push.offer_panel(session, completion, Panels::All, false);
    push.offer_panel(session, record(81, 10, dated::DATA, &[2; 8]), Panels::One(81), false);
    push.offer_panel(session, record(82, 10, dated::DATA, &[3; 8]), Panels::One(82), true);
    assert!(push.try_take().is_none());
    push.credit(session, completion_charge as u64);
    let (_, bytes, ..) = push.try_take().expect("partial credit lets the cell-free completion pass the FOLLOW");
    assert_eq!(sent(&bytes), [(0, 11, dated::COMPLETED)]);
    let (_, bytes, ..) = push.try_take().expect("cell-free panel 81 then passes the FOLLOW");
    assert_eq!(sent(&bytes), [(81, 10, dated::DATA)]);
    assert!(push.try_take().is_none(), "live-cell panel 82 still waits behind the FOLLOW");
}

#[test]
fn a_seed_panel_leaves_at_its_data_hour_while_a_waiting_only_packet_waits() {
    use crate::{panel::dated, push::{Panels, Push}};
    let record = |id, hour, kind, body: &[u8]| dated::encoded(0, 3, 0, id, hour, kind, body);
    let sent = |bytes: &[u8]| dated::records(bytes).unwrap().into_iter().map(|r| (r.id, r.hour, r.kind)).collect::<Vec<_>>();
    let held = record(39, 5, dated::DATA, &[1; 1000]);
    let one = crate::answer::charge(&held);
    let push = Push::with_budget(playhead::ByteBudget::new(one * 3));
    let session = push.session();
    push.head(3, 0, 10);
    push.offer_panel(session, held, Panels::One(39), false);
    assert!(push.try_take().is_some());
    push.offer_panel(session, record(38, 10, dated::DATA, &[2; 1000]), Panels::One(38), false);
    assert!(push.try_take().is_some(), "reached record fills past credit while budget has room");
    let seed = dated::at_one_hour((0, 3, 0), 44, 10, dated::DATA, &[7; 8]);
    assert!(push.in_flight() + crate::answer::charge(&seed) > one * 3 / 2, "credit is spent");
    push.offer_panel(session, seed, Panels::One(44), false);
    let (_, bytes, ..) = push.try_take().expect("a seed for the hour on screen leaves although its WAITING carries the next hour");
    assert_eq!(sent(&bytes), [(44, 10, dated::DATA), (44, 11, dated::WAITING)]);
    push.offer_panel(session, record(45, 11, dated::WAITING, &[]), Panels::One(45), false);
    assert!(push.try_take().is_none(), "a WAITING-only packet for hour 11 stays unreached at hour 10");
    push.head(3, 0, 11);
    let (_, bytes, ..) = push.try_take().expect("and leaves once the clock reaches hour 11");
    assert_eq!(sent(&bytes), [(45, 11, dated::WAITING)]);
}

#[test]
fn fol3_stopped_round_trips_and_matches_ts_golden() {
    use crate::push::OVERFLOW_REASON;
    let golden = "464f4c3300000000180000000100000002000000000000000700000018000000580000000000000004000000000000000000000001000000000000003a0000005468697320706167652066656c6c20746f6f2066617220626568696e64207768696c6520697420776173206e6f7420726573706f6e64696e672e000000000000";
    let mut body = Vec::new();
    crate::answer::stopped(OVERFLOW_REASON, &mut body);
    let mut packet = Vec::new();
    crate::answer::begin(&mut packet, 0, 0, 24, 2, false);
    crate::answer::push(&mut packet, 7, 24, &body);
    let hex: String = packet.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, golden, "Rust FOL3 STOPPED wire bytes match the TypeScript golden fixture");
    let decoded = decode(&packet).unwrap();
    assert_eq!((decoded.route, decoded.hour, decoded.stamp.0, decoded.reset), (0, 24, 2, false));
    assert_eq!(decoded.answers.len(), 1);
    assert_eq!((decoded.answers[0].0, decoded.answers[0].1), (7, 24));
    let parsed = Answer::parse(decoded.answers[0].2).unwrap();
    assert_eq!(parsed.stopped.as_deref(), Some(OVERFLOW_REASON));
    assert_eq!(parsed.not_recorded, None);
}

#[test]
fn overflowing_route_drops_and_fails_with_stopped_while_sibling_route_keeps_playing_and_retry_matches_fresh() {
    use crate::{
        host::PlayheadHost,
        panel::{self, AtHour, PanelReader, PanelRows, ProductRead, dated},
        push::{FOLLOW, OVERFLOW_REASON, PANEL, Push},
        registry::{Anchor, Hosted},
    };
    struct IdleRows;
    impl PanelRows for IdleRows {
        fn reader(&self, _: &[replay_data::rows::TableId]) -> io::Result<Box<dyn PanelReader>> { unreachable!() }
        fn on_publish(&self, _: panel::Wake) {}
    }
    const PANEL_PAYLOAD_BYTES: usize = 1536;
    let counter_panel = || {
        let advance: panel::ProductReader = Arc::new(|hour, _, _, _| ProductRead {
            value: Arc::new(Ok(serde_json::json!({ "hour": hour, "payload": "p".repeat(PANEL_PAYLOAD_BYTES) }))),
            rows: 1,
            fresh: true,
            pending: false,
            live: None,
            lanes: Vec::new(),
            from: Hour::MIN,
        });
        let read: panel::ExactReader = Arc::new(|hour, _| Ok(serde_json::json!({ "hour": hour, "payload": "p".repeat(PANEL_PAYLOAD_BYTES) })));
        Box::new(AtHour { advance, cold: Arc::clone(&read), read })
    };

    let pages = record(quiet);
    let live_query = Query::Cells { table: TABLE, columns: vec![0, 2, 3], rows: Rows::Live(vec![100, 105, 106, 107]) };
    let all_query = Query::Cells { table: TABLE, columns: vec![0, 3], rows: Rows::All };
    let sibling_query = Query::Cells { table: TABLE, columns: vec![0], rows: Rows::Keys(vec![100]) };

    let mut fresh_harness = harness(&pages);
    fresh_harness.follow.follow(1, live_query.clone()).unwrap();
    fresh_harness.follow.follow(2, all_query.clone()).unwrap();

    let budget = playhead::ByteBudget::new(32_768);
    let push = Arc::new(Push::with_budget(budget.clone()));
    let session = push.session();
    let follow_a = Follow::new(0);
    let follow_b = Follow::new(0);
    let route_a = follow_a.route();
    let route_b = follow_b.route();
    assert_ne!(route_a, route_b);

    let pace_gate = Arc::new((Mutex::new((0usize, 0usize)), std::sync::Condvar::new()));
    let offer_gate = Arc::clone(&pace_gate);
    let a_joined = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let b_joined = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (a_ready, b_ready) = (Arc::clone(&a_joined), Arc::clone(&b_joined));
    let (push_a, push_b) = (Arc::clone(&push), Arc::clone(&push));
    let mut hook_a = follow_a.hook(Box::new(move |stats, message| {
        if let Some(message) = message {
            if message.answers == 2 { a_ready.store(true, std::sync::atomic::Ordering::Release); }
            push_a.offer_follow(stats.session, message.bytes);
        }
    }));
    let mut hook_b = follow_b.hook(Box::new(move |stats, message| {
        if let Some(message) = message {
            if message.answers == 1 { b_ready.store(true, std::sync::atomic::Ordering::Release); }
            let mut counts = offer_gate.0.lock().unwrap();
            while counts.0 >= counts.1.saturating_add(50) {
                counts = offer_gate.1.wait(counts).unwrap();
            }
            counts.0 += 1;
            drop(counts);
            push_b.offer_follow(stats.session, message.bytes);
        }
    }));
    struct GateGuard(Arc<(Mutex<(usize, usize)>, std::sync::Condvar)>);
    impl Drop for GateGuard {
        fn drop(&mut self) {
            self.0.0.lock().unwrap_or_else(|e| e.into_inner()).1 = usize::MAX / 2;
            self.0.1.notify_all();
        }
    }
    let combined: playhead::Hook = Box::new(move |step, lanes| {
        hook_a(step, lanes);
        hook_b(step, lanes);
    });
    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: RING, speed: 1.0, rate: 60.0, ..Settings::default() };
    let playhead = Arc::new(Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings, Some(combined)).unwrap());
    let _gate_guard = GateGuard(Arc::clone(&pace_gate));
    let held = crate::host::held_tables(&*pages, pages.kinds.len());
    let anchored = |p: &Playhead, a: Anchor| {
        if !a.playing { p.pace(a.speed); p.pause(); }
        if a.sought { p.seek(a.hour); }
        if a.playing { p.play(a.speed); }
    };
    let host_a = PlayheadHost::new_with_budget(Arc::clone(&playhead), |p| p, anchored, follow_a, held.clone(), budget.clone()).unwrap();
    let host_b = PlayheadHost::new_with_budget(Arc::clone(&playhead), |p| p, anchored, follow_b, held, budget.clone()).unwrap();
    host_a.show_panels(Arc::new(IdleRows), Arc::clone(&push));
    host_b.show_panels(Arc::new(IdleRows), Arc::clone(&push));
    host_a.serve(session);
    host_b.serve(session);

    host_a.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
    host_b.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: false });
    host_a.follow().follow(1, live_query.clone()).unwrap();
    host_a.follow().follow(2, all_query.clone()).unwrap();
    host_b.follow().follow(3, sibling_query).unwrap();
    while !(a_joined.load(std::sync::atomic::Ordering::Acquire) && b_joined.load(std::sync::atomic::Ordering::Acquire)) {
        std::thread::yield_now();
    }
    host_a.prepare_panel(counter_panel()).unwrap()(10, 1).unwrap();
    host_a.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: true, speed: 60.0, sought: false });
    host_b.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: true, speed: 60.0, sought: false });

    let epoch_before = playhead.epoch();
    while push.counters().dropped == 0 {
        std::thread::yield_now();
    }
    let mut credited = 0u64;
    let mut b_hours = Vec::new();
    let mut saw_panel_stopped = false;
    let mut saw_follow_stopped = std::collections::HashSet::new();
    let mut extra_a_after_stopped = 0usize;

    let mut settled_b_after_stop = 0usize;
    loop {
        let (kind, bytes, sent_session) = push.take_sent().expect("stream packet");
        assert_eq!(sent_session, session);
        let charge = crate::answer::charge(&bytes) as u64;
        credited += charge;
        push.credit(session, credited);
        if kind == PANEL {
            let (route, seek) = dated::stamp(&bytes).unwrap();
            assert_eq!((route, seek), (route_a, 1));
            let records = dated::records(&bytes).unwrap();
            let has_stop = records.iter().any(|r| r.id == 10 && r.kind == dated::STOPPED && r.hour == Hour::MIN && r.body == OVERFLOW_REASON.as_bytes())
                && records.iter().any(|r| r.id == 10 && r.kind == dated::STANDS && r.hour == Hour::MAX);
            if has_stop {
                saw_panel_stopped = true;
            } else if saw_panel_stopped {
                extra_a_after_stopped += 1;
            }
        } else {
            assert_eq!(kind, FOLLOW);
            let decoded = decode(&bytes).unwrap();
            if decoded.route == route_a {
                let mut packet_stopped = false;
                for (id, _, body) in decoded.answers {
                    if let Ok(parsed) = Answer::parse(body) && parsed.stopped.as_deref() == Some(OVERFLOW_REASON) {
                        saw_follow_stopped.insert(id);
                        packet_stopped = true;
                    }
                }
                if !packet_stopped && saw_follow_stopped.len() == 2 {
                    extra_a_after_stopped += 1;
                }
            } else {
                assert_eq!(decoded.route, route_b);
                for (_, _, body) in &decoded.answers {
                    let parsed = Answer::parse(body).unwrap();
                    assert_eq!(parsed.stopped, None, "healthy sibling route B must never receive STOPPED");
                }
                b_hours.push(decoded.hour);
                {
                    let mut counts = pace_gate.0.lock().unwrap();
                    counts.1 += 1;
                    pace_gate.1.notify_all();
                }
                if saw_panel_stopped && saw_follow_stopped.len() == 2 {
                    settled_b_after_stop += 1;
                    if settled_b_after_stop >= 5 { break; }
                }
            }
        }
    }

    assert!(saw_panel_stopped, "overflowed route delivered PNL2 STOPPED + STANDS(MAX) for panel 10");
    assert_eq!(saw_follow_stopped, std::collections::HashSet::from([1u32, 2u32]), "overflowed route delivered FOL3 STOPPED for every ordinary follow");
    assert_eq!(extra_a_after_stopped, 0, "overflowed route produces nothing more after STOPPED");
    assert!(b_hours.len() >= 5, "sibling route B kept receiving continuously while route A failed: {b_hours:?}");
    assert_eq!(playhead.epoch(), epoch_before, "route overflow and teardown never touch playhead.epoch()");
    assert!(push.counters().dropped > 0);

    host_a.serve(session);
    host_a.prepare_panel(counter_panel()).unwrap()(11, 1).unwrap();
    host_a.follow_query(4, live_query.clone()).unwrap();
    let (mut late_panel_stop, mut late_follow_stop) = (false, false);
    while let Some((kind, bytes, ..)) = push.try_take() {
        if kind == PANEL {
            let (route, seek) = dated::stamp(&bytes).unwrap();
            if route == route_a && seek == 1 && dated::records(&bytes).unwrap().iter().any(|r| r.id == 11 && r.kind == dated::STOPPED && r.body == OVERFLOW_REASON.as_bytes()) {
                late_panel_stop = true;
            }
        } else {
            let decoded = decode(&bytes).unwrap();
            if decoded.route == route_a && decoded.stamp.0 == 1
                && decoded.answers.iter().any(|&(id, _, body)| id == 4 && Answer::parse(body).ok().and_then(|a| a.stopped).as_deref() == Some(OVERFLOW_REASON))
            {
                late_follow_stop = true;
            }
        }
    }
    assert!(late_panel_stop && late_follow_stop, "late panel and follow registrations on an overflowed session immediately receive stream STOPPED for Retry");

    host_a.anchor(Anchor { seek: 2, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
    let (mut seek2_panel_stops, mut seek2_follow_stops) = (std::collections::HashSet::new(), std::collections::HashSet::new());
    while let Some((kind, bytes, ..)) = push.try_take() {
        if kind == PANEL {
            let (route, seek) = dated::stamp(&bytes).unwrap();
            if route == route_a && seek == 2 {
                for r in dated::records(&bytes).unwrap() {
                    if r.kind == dated::STOPPED && r.body == OVERFLOW_REASON.as_bytes() {
                        seek2_panel_stops.insert(r.id);
                    }
                }
            }
        } else {
            let decoded = decode(&bytes).unwrap();
            if decoded.route == route_a && decoded.stamp.0 == 2 {
                for (id, _, body) in decoded.answers {
                    if let Ok(parsed) = Answer::parse(body) && parsed.stopped.as_deref() == Some(OVERFLOW_REASON) {
                        seek2_follow_stops.insert(id);
                    }
                }
            }
        }
    }
    assert_eq!(seek2_panel_stops, std::collections::HashSet::from([10u32, 11u32]), "a seek on an overflowed route synchronously re-emits PNL2 STOPPED at the new seek");
    assert_eq!(seek2_follow_stops, std::collections::HashSet::from([1u32, 2u32, 4u32]), "a seek on an overflowed route synchronously re-emits FOL3 STOPPED at the new seek");

    {
        pace_gate.0.lock().unwrap().1 = usize::MAX / 2;
        pace_gate.1.notify_all();
    }
    let retry_hour = *b_hours.last().unwrap();
    let new_session = push.restart();
    host_a.serve(new_session);
    host_b.serve(new_session);
    host_a.anchor(Anchor { seek: 3, frame_rate: 60.0, hour: retry_hour, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
    host_a.follow().follow(1, live_query.clone()).unwrap();
    host_a.follow().follow(2, all_query.clone()).unwrap();
    host_a.prepare_panel(counter_panel()).unwrap()(10, 1).unwrap();

    let mut retry_follow: std::collections::HashMap<u32, (Hour, Vec<u8>)> = std::collections::HashMap::new();
    let mut retry_panel: Option<(Hour, Vec<u8>)> = None;
    let mut released = 0u64;
    while retry_follow.len() < 2 || retry_panel.is_none() {
        let (kind, bytes, sent_session) = push.take_sent().expect("retry packet");
        assert_eq!(sent_session, new_session);
        released += crate::answer::charge(&bytes) as u64;
        push.credit(new_session, released);
        if kind == PANEL {
            let (route, seek) = dated::stamp(&bytes).unwrap();
            if route == route_a && seek == 3 {
                for r in dated::records(&bytes).unwrap() {
                    assert_ne!(r.kind, dated::STOPPED, "retry panel must not receive STOPPED");
                    if r.id == 10 && r.kind == dated::DATA && r.hour == retry_hour {
                        retry_panel = Some((r.hour, r.body.to_vec()));
                    }
                }
            }
        } else {
            let decoded = decode(&bytes).unwrap();
            if decoded.route == route_a && decoded.stamp.0 == 3 && decoded.hour == retry_hour {
                for (id, h, body) in decoded.answers {
                    assert_eq!(Answer::parse(body).unwrap().stopped, None, "retry follow must not receive STOPPED");
                    if id == 1 || id == 2 {
                        retry_follow.insert(id, (h, body.to_vec()));
                    }
                }
            }
        }
    }

    fresh_harness.seek(retry_hour);
    let fresh_played = fresh_harness.play(retry_hour);
    let fresh_bytes = fresh_played.last().unwrap().message.as_ref().unwrap().bytes.clone();
    let fresh_decoded = decode(&fresh_bytes).unwrap();
    for id in [1u32, 2u32] {
        let fresh = fresh_decoded.answers.iter().find(|&&(qid, ..)| qid == id).expect("fresh follow answer");
        assert_eq!(retry_follow[&id], (fresh.1, fresh.2.to_vec()), "retry follow {id} matches fresh open byte for byte");
    }
    let expected_panel_body = rmp_serde::to_vec(&serde_json::json!({ "hour": retry_hour, "payload": "p".repeat(PANEL_PAYLOAD_BYTES) })).unwrap();
    assert_eq!(retry_panel, Some((retry_hour, expected_panel_body)), "retry panel 10 matches fresh open byte for byte");
}

#[test]
fn route_queued_bytes_stay_within_budget_under_stalled_writer_or_zero_credit() {
    use crate::push::Push;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let sample = rows_message(10, false, &[(1, 1)]);
    let one = crate::answer::charge(&sample);
    let limit = one * 3;
    let budget = playhead::ByteBudget::new(limit);
    let push = Push::with_budget(budget.clone());
    let overflows = Arc::new(AtomicUsize::new(0));
    let hit = Arc::clone(&overflows);
    push.watch_overflow(0, Arc::new(move |_| { hit.fetch_add(1, Ordering::AcqRel); }));

    push.head(0, 0, 10);
    push.offer(sample);
    assert!(push.try_take().is_some(), "first packet goes out before the pipe writer stalls");
    for hour in 11..200 {
        push.head(0, 0, hour);
        push.offer(rows_message(hour, false, &[(1, i64::from(hour))]));
        let queued = budget.used().saturating_sub(push.in_flight());
        assert!(queued <= limit, "stalled writer hour {hour}: queued {queued} exceeded limit {limit}");
    }
    assert_eq!(overflows.load(Ordering::Acquire), 1, "overflow hook fires once on the stalled route");
    assert!(push.counters().dropped > 0);
    assert_eq!(budget.used().saturating_sub(push.in_flight()), 0, "dropped backlog leaves zero queued bytes");

    let limit = one * 2;
    let budget = playhead::ByteBudget::new(limit);
    let push = Push::with_budget(budget.clone());
    let zero_overflows = Arc::new(AtomicUsize::new(0));
    let hit = Arc::clone(&zero_overflows);
    push.watch_overflow(0, Arc::new(move |_| { hit.fetch_add(1, Ordering::AcqRel); }));
    for hour in 10..200 {
        push.head(0, 0, hour);
        push.offer(rows_message(hour, false, &[(1, i64::from(hour))]));
        while push.try_take().is_some() {}
        let queued = budget.used().saturating_sub(push.in_flight());
        assert!(queued <= limit, "zero-credit hour {hour}: queued {queued} exceeded limit {limit}");
    }
    assert_eq!(zero_overflows.load(Ordering::Acquire), 1, "zero-credit consumer hits queue bound and trips overflow once");
    assert!(push.counters().dropped > 0);
}

#[test]
fn push_queue_commute_ordering_matches_serial_delivery_across_generated_sequences() {
    use crate::{panel::dated, push::{FOLLOW, PANEL, Panels, Push}};
    use std::collections::{BTreeMap, HashMap, HashSet};

    const CELL_1: u32 = crate::live::CELLS | 100;
    const CELL_2: u32 = crate::live::CELLS | 200;
    const CELL_3: u32 = crate::live::CELLS | 300;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum PanelVal {
        Data(i32, Vec<u32>),
        Refused(i32),
        Stopped(i32),
    }

    #[derive(Clone, Debug)]
    struct PanelFact {
        h: Hour,
        c: Hour,
        val: PanelVal,
    }

    #[derive(Clone, Debug, Default)]
    struct PanelSub {
        facts: Vec<PanelFact>,
        open: bool,
        stepped: bool,
        failed: bool,
    }

    #[derive(Clone, Debug)]
    struct FollowFact {
        h: Hour,
        c: Hour,
        rows: BTreeMap<u128, i64>,
    }

    #[derive(Clone, Debug, Default)]
    struct FollowOwner {
        facts: Vec<FollowFact>,
        open: bool,
        failed: bool,
    }

    #[derive(Clone, Debug)]
    struct HeldItem {
        id: u32,
        hour: Hour,
        answers: Vec<(Hour, Vec<u8>)>,
        limited: Option<Hour>,
        size: usize,
    }

    struct PageModel {
        early_limit: usize,
        per_cell: bool,
        panels: HashMap<u32, PanelSub>,
        followed: HashMap<u32, FollowOwner>,
        groups: HashMap<u32, FollowOwner>,
        held: Vec<HeldItem>,
        held_bytes: usize,
        held_by_cell: HashMap<u32, usize>,
        lost: HashSet<u32>,
        ever_overflowed: bool,
    }

    impl PageModel {
        fn new(early_limit: usize, per_cell: bool) -> Self {
            let mut panels = HashMap::new();
            for id in [1, 2, 3] { panels.insert(id, PanelSub::default()); }
            let mut followed = HashMap::new();
            for id in [10, 11] { followed.insert(id, FollowOwner::default()); }
            Self {
                early_limit,
                per_cell,
                panels,
                followed,
                groups: HashMap::new(),
                held: Vec::new(),
                held_bytes: 0,
                held_by_cell: HashMap::new(),
                lost: HashSet::new(),
                ever_overflowed: false,
            }
        }

        fn encode_data(val: i32, cells: &[u32]) -> Vec<u8> {
            let mut out = Vec::with_capacity(8 + cells.len() * 4);
            out.extend_from_slice(&val.to_le_bytes());
            out.extend_from_slice(&(cells.len() as u32).to_le_bytes());
            for &cell in cells {
                out.extend_from_slice(&cell.to_le_bytes());
            }
            out
        }

        fn decode_data(body: &[u8]) -> (i32, Vec<u32>) {
            let val = i32::from_le_bytes(body[..4].try_into().expect("val"));
            let count = u32::from_le_bytes(body[4..8].try_into().expect("count")) as usize;
            let mut cells = Vec::with_capacity(count);
            for i in 0..count {
                let at = 8 + i * 4;
                cells.push(u32::from_le_bytes(body[at..at + 4].try_into().expect("cell")));
            }
            (val, cells)
        }

        fn add_panel_fact(sub: &mut PanelSub, fact: PanelFact) {
            if let Some(last) = sub.facts.last_mut() {
                if last.h == fact.h {
                    *last = fact;
                } else {
                    if last.c >= fact.h { last.c = fact.h - 1; }
                    sub.facts.push(fact);
                }
            } else {
                sub.facts.push(fact);
            }
            sub.open = true;
        }

        fn add_follow_fact(owner: &mut FollowOwner, h: Hour, body: &[u8]) {
            if owner.failed { return; }
            let parsed = Answer::parse(body).expect("valid answer");
            let rows = if parsed.changed {
                let Some(prev) = owner.facts.last() else {
                    owner.failed = true;
                    owner.facts.clear();
                    return;
                };
                let mut merged = prev.rows.clone();
                for (i, &k) in parsed.keys.iter().enumerate() {
                    if !merged.contains_key(&k) {
                        owner.failed = true;
                        owner.facts.clear();
                        return;
                    }
                    merged.insert(k, parsed.values[i]);
                }
                merged
            } else {
                parsed.keys.iter().copied().zip(parsed.values.iter().copied()).collect()
            };
            let fact = FollowFact { h, c: h, rows };
            if let Some(last) = owner.facts.last_mut() {
                if last.h == fact.h {
                    *last = fact;
                } else {
                    if last.c >= fact.h { last.c = fact.h - 1; }
                    owner.facts.push(fact);
                }
            } else {
                owner.facts.push(fact);
            }
            owner.open = true;
        }

        fn hold_cell(&mut self, item: HeldItem) {
            let (id, size) = (item.id, item.size);
            if self.lost.contains(&id) { return; }
            let current = if self.per_cell { self.held_by_cell.get(&id).copied().unwrap_or(0) } else { self.held_bytes };
            if current + size + 32 > self.early_limit {
                self.held.retain(|existing| {
                    if existing.id == id {
                        self.held_bytes -= existing.size + 32;
                        if let Some(entry) = self.held_by_cell.get_mut(&id) {
                            *entry -= existing.size + 32;
                        }
                        false
                    } else {
                        true
                    }
                });
                self.lost.insert(id);
                self.ever_overflowed = true;
                return;
            }
            self.held_bytes += size + 32;
            *self.held_by_cell.entry(id).or_insert(0) += size + 32;
            self.held.push(item);
        }

        fn drain_cell(&mut self, cell: u32) {
            let mut ready = Vec::new();
            let mut kept = Vec::new();
            for item in self.held.drain(..) {
                if item.id == cell {
                    self.held_bytes -= item.size + 32;
                    if let Some(entry) = self.held_by_cell.get_mut(&cell) {
                        *entry -= item.size + 32;
                    }
                    ready.push(item);
                } else {
                    kept.push(item);
                }
            }
            self.held = kept;
            for item in ready {
                if let Some(owner) = self.groups.get_mut(&item.id) {
                    for (at, body) in item.answers {
                        Self::add_follow_fact(owner, at, &body);
                    }
                    if owner.open && let Some(last) = owner.facts.last_mut() {
                        let confirmed = item.limited.unwrap_or(item.hour);
                        if confirmed > last.c { last.c = confirmed; }
                    }
                }
            }
        }

        fn apply(&mut self, kind: u8, bytes: &[u8]) {
            if kind == PANEL {
                let Ok(records) = dated::records(bytes) else { return };
                for r in records {
                    if r.kind == dated::COMPLETED {
                        for sub in self.panels.values_mut() {
                            if !sub.failed && sub.stepped && sub.open && let Some(last) = sub.facts.last_mut() && last.c < r.hour {
                                last.c = r.hour;
                            }
                        }
                        continue;
                    }
                    let Some(sub) = self.panels.get_mut(&r.id) else { continue };
                    if sub.failed { continue; }
                    if matches!(r.kind, dated::DATA | dated::REFUSED | dated::STOPPED | dated::STANDS | dated::AGAIN) && r.stepped() {
                        sub.stepped = true;
                    }
                    match r.kind {
                        dated::DATA => {
                            let (val, cells) = Self::decode_data(r.body);
                            if cells.iter().any(|cell| self.lost.contains(cell)) {
                                sub.failed = true;
                                sub.facts.clear();
                                sub.open = false;
                                continue;
                            }
                            for &cell in &cells {
                                self.groups.entry(cell).or_default();
                            }
                            Self::add_panel_fact(sub, PanelFact { h: r.hour, c: r.hour, val: PanelVal::Data(val, cells.clone()) });
                            for cell in cells {
                                self.drain_cell(cell);
                            }
                        }
                        dated::REFUSED => {
                            let code = i32::from_le_bytes(r.body[..4].try_into().expect("code"));
                            Self::add_panel_fact(sub, PanelFact { h: r.hour, c: r.hour, val: PanelVal::Refused(code) });
                        }
                        dated::STOPPED => {
                            let code = i32::from_le_bytes(r.body[..4].try_into().expect("code"));
                            Self::add_panel_fact(sub, PanelFact { h: r.hour, c: r.hour, val: PanelVal::Stopped(code) });
                        }
                        dated::AGAIN => {
                            if let Some(last) = sub.facts.last().cloned() {
                                Self::add_panel_fact(sub, PanelFact { h: r.hour, c: r.hour, val: last.val });
                            }
                        }
                        dated::STANDS => {
                            if let Some(last) = sub.facts.last_mut() {
                                last.c = last.c.max(r.hour);
                                sub.open = true;
                            }
                        }
                        dated::WAITING => {
                            sub.open = false;
                        }
                        _ => {}
                    }
                }
            } else if kind == FOLLOW {
                let Ok(decoded) = decode(bytes) else { return };
                if decoded.reset {
                    self.lost.clear();
                    self.held.clear();
                    self.held_bytes = 0;
                    self.held_by_cell.clear();
                    for owner in self.followed.values_mut().chain(self.groups.values_mut()) {
                        owner.facts.clear();
                        owner.open = false;
                    }
                }
                let mut limited_map: HashMap<u32, Hour> = HashMap::new();
                for &(id, at) in &decoded.limited {
                    if id >= crate::live::CELLS && !self.groups.contains_key(&id) {
                        continue;
                    }
                    limited_map.insert(id, at);
                }
                let mut unbound: HashMap<u32, (Vec<(Hour, Vec<u8>)>, Option<Hour>, usize)> = HashMap::new();
                let mut unbound_order: Vec<u32> = Vec::new();
                let count = u32::from_le_bytes(bytes[12..16].try_into().expect("count")) as usize;
                let mut at = 24usize;
                for _ in 0..count {
                    let id = u32::from_le_bytes(bytes[at..at + 4].try_into().expect("id"));
                    let hour = i32::from_le_bytes(bytes[at + 4..at + 8].try_into().expect("hour"));
                    let len = u32::from_le_bytes(bytes[at + 8..at + 12].try_into().expect("len")) as usize;
                    let whole = u32::from_le_bytes(bytes[at + 12..at + 16].try_into().expect("whole")) as usize;
                    let body = &bytes[at + 16..at + 16 + len];
                    at += 16 + len;
                    let charge = 16 + 2 * len.max(whole);
                    if len == 0 {
                        if id >= crate::live::CELLS && !self.groups.contains_key(&id) {
                            if !unbound.contains_key(&id) { unbound_order.push(id); }
                            let entry = unbound.entry(id).or_default();
                            entry.1 = Some(hour);
                            entry.2 += charge;
                        }
                    } else if id < crate::live::CELLS {
                        if let Some(owner) = self.followed.get_mut(&id) {
                            Self::add_follow_fact(owner, hour, body);
                        }
                    } else if let Some(owner) = self.groups.get_mut(&id) {
                        Self::add_follow_fact(owner, hour, body);
                    } else {
                        if !unbound.contains_key(&id) { unbound_order.push(id); }
                        let entry = unbound.entry(id).or_default();
                        entry.0.push((hour, body.to_vec()));
                        entry.2 += charge;
                    }
                }
                let mut held_ids = unbound_order;
                for item in &self.held {
                    if !held_ids.contains(&item.id) { held_ids.push(item.id); }
                }
                for id in held_ids {
                    let (answers, limited, size) = unbound.remove(&id).unwrap_or_default();
                    self.hold_cell(HeldItem { id, hour: decoded.hour, answers, limited, size });
                }
                for (&id, owner) in self.followed.iter_mut().chain(self.groups.iter_mut()) {
                    if !owner.open || owner.failed { continue; }
                    if let Some(last) = owner.facts.last_mut() {
                        let confirmed = limited_map.get(&id).copied().unwrap_or(decoded.hour);
                        if confirmed > last.c { last.c = confirmed; }
                    }
                }
            }
        }

        fn covering_panel(&self, id: u32, hour: Hour) -> Option<(PanelVal, Vec<BTreeMap<u128, i64>>)> {
            let sub = self.panels.get(&id)?;
            let fact = sub.facts.iter().rev().find(|f| f.h <= hour && hour <= f.c)?;
            let mut cell_rows = Vec::new();
            if let PanelVal::Data(_, cells) = &fact.val {
                for cell in cells {
                    let group = self.groups.get(cell)?;
                    let gf = group.facts.iter().rev().find(|f| f.h <= hour && hour <= f.c)?;
                    cell_rows.push(gf.rows.clone());
                }
            }
            Some((fact.val.clone(), cell_rows))
        }

        fn covering_follow(&self, id: u32, hour: Hour) -> Option<&BTreeMap<u128, i64>> {
            let owner = self.followed.get(&id)?;
            owner.facts.iter().rev().find(|f| f.h <= hour && hour <= f.c).map(|f| &f.rows)
        }
    }

    let make_answer = |changed: bool, rows: &[(u128, i64)]| -> Vec<u8> {
        let mut table = crate::answer::Table::default();
        table.clear(1, false);
        for &(k, v) in rows {
            table.row(k);
            table.cell(v, State::Value);
        }
        let mut out = Vec::new();
        if changed { table.encode_changed(&mut out); } else { table.encode(&mut out); }
        out
    };

    let seeds = [0x5EED_6565u64, 0xCAFE_6565, 0xBEEF_6565, 0xDEAD_6565];
    let early_limits = [200usize, 270, 300, 500, 1 << 20];
    let budgets = [2048usize, 4096, 16384, 1 << 20];
    let mut total_overtakes = 0u64;
    let mut checked_trials = 0u64;

    for seed in seeds {
        let mut rng_state = seed;
        let mut next_u32 = || -> u32 {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (rng_state >> 32) as u32
        };

        for trial in 0..300u32 {
            let early_limit = early_limits[(next_u32() as usize) % early_limits.len()];
            let budget_bytes = budgets[(next_u32() as usize) % budgets.len()];
            let per_cell = trial % 2 == 1;
            let ctx = format!("seed={seed:#x} trial={trial} early_limit={early_limit} budget={budget_bytes} per_cell={per_cell}");
            let budget = playhead::ByteBudget::new(budget_bytes);
            let push = Push::with_budget(budget.clone());
            let session = push.session();
            let mut ref_model = PageModel::new(early_limit, per_cell);
            let mut q_model = PageModel::new(early_limit, per_cell);

            let mut panel_has_data = [false; 4];
            let mut panel_stepped = [false; 4];
            let mut panel_hour = [9i32; 4];
            let mut follow_has_base: HashMap<u32, bool> = HashMap::new();
            let mut step_hour = 10i32;
            let mut last_follow_hour = 9i32;
            let mut max_head = -1i32;
            let mut total_sent = 0u64;
            let mut last_seq_taken = 0u32;
            let mut trial_overtakes = 0u64;

            let steps = 15 + (next_u32() % 25);
            for _ in 0..steps {
                step_hour += 1 + (next_u32() % 3) as i32;
                let roll = next_u32() % 100;
                if roll < 20 {
                    let h = max_head.max((step_hour + 2).min(step_hour - (next_u32() % 8) as i32));
                    max_head = max_head.max(h);
                    push.head(0, 0, h);
                } else if roll < 36 {
                    if let Some((kind, bytes, seq, _)) = push.try_take() {
                        if kind == FOLLOW && !bytes.starts_with(&crate::answer::MAGIC.to_le_bytes()) {
                            continue;
                        }
                        if seq < last_seq_taken { trial_overtakes += 1; }
                        last_seq_taken = last_seq_taken.max(seq);
                        total_sent += crate::answer::charge(&bytes) as u64;
                        q_model.apply(kind, &bytes);
                        if next_u32() % 2 == 0 { push.credit(session, total_sent); }
                    }
                } else if roll < 43 {
                    if total_sent > 0 {
                        push.credit(session, (u64::from(next_u32()) % total_sent).min(total_sent));
                    }
                } else if roll < 46 {
                    push.offer(vec![0u8; 8]);
                } else if roll < 74 {
                    if next_u32() % 100 < 24 {
                        let h = step_hour.max(panel_hour[1] + 1).max(panel_hour[2] + 1).max(panel_hour[3] + 1);
                        step_hour = h;
                        let mut batch = dated::ProductBatch::new((0, 0, 0));
                        let mut has_cells = false;
                        for pid in [1u32, 2, 3] {
                            if next_u32() % 100 < 35 {
                                panel_hour[pid as usize] = h;
                                let k = next_u32() % 100;
                                if k < 50 {
                                    let cells: &[u32] = match pid {
                                        1 => match next_u32() % 100 {
                                            0..35 => &[CELL_1],
                                            35..50 => &[CELL_1, CELL_3],
                                            _ => &[],
                                        },
                                        2 if next_u32() % 100 < 40 => &[CELL_2],
                                        _ => &[],
                                    };
                                    let body = PageModel::encode_data(pid as i32 * 1000 + h, cells);
                                    batch.changed(pid, h, dated::DATA, &body);
                                    panel_has_data[pid as usize] = true;
                                    panel_stepped[pid as usize] = true;
                                    has_cells |= !cells.is_empty();
                                } else if k < 70 && panel_has_data[pid as usize] {
                                    batch.changed(pid, h, dated::STANDS, &[]);
                                    panel_stepped[pid as usize] = true;
                                } else if k < 85 && panel_has_data[pid as usize] {
                                    batch.changed(pid, h, dated::AGAIN, &[]);
                                    panel_stepped[pid as usize] = true;
                                } else {
                                    batch.changed(pid, h, dated::WAITING, &[]);
                                }
                            }
                        }
                        let pkt = batch.finish(Some(h));
                        ref_model.apply(PANEL, &pkt);
                        push.offer_panel(session, pkt, Panels::All, has_cells);
                    } else {
                        let unstepped: Vec<u32> = [1, 2, 3].into_iter().filter(|&id| !panel_stepped[id as usize]).collect();
                        if unstepped.is_empty() { continue; }
                        let pid = unstepped[(next_u32() as usize) % unstepped.len()];
                        let future = if next_u32() % 100 < 30 { 12 } else { 0 };
                        let h = (panel_hour[pid as usize] + 1).max(step_hour + future);
                        panel_hour[pid as usize] = h;
                        let kr = next_u32() % 100;
                        let mut has_cells = false;
                        let pkt = if kr < 45 {
                            let cells: &[u32] = match pid {
                                1 => match next_u32() % 100 {
                                    0..35 => &[CELL_1],
                                    35..50 => &[CELL_1, CELL_3],
                                    _ => &[],
                                },
                                2 if next_u32() % 100 < 45 => &[CELL_2],
                                _ => &[],
                            };
                            has_cells = !cells.is_empty();
                            panel_has_data[pid as usize] = true;
                            let body = PageModel::encode_data(pid as i32 * 1000 + h, cells);
                            if next_u32() % 100 < 45 {
                                dated::at_one_hour((0, 0, 0), pid, h, dated::DATA, &body)
                            } else {
                                dated::encoded(0, 0, 0, pid, h, dated::DATA, &body)
                            }
                        } else if kr < 65 && panel_has_data[pid as usize] {
                            dated::encoded(0, 0, 0, pid, h, dated::STANDS, &[])
                        } else if kr < 78 && panel_has_data[pid as usize] {
                            dated::encoded(0, 0, 0, pid, h, dated::AGAIN, &[])
                        } else if kr < 88 {
                            dated::encoded(0, 0, 0, pid, h, dated::WAITING, &[])
                        } else if kr < 94 {
                            panel_has_data[pid as usize] = true;
                            dated::encoded(0, 0, 0, pid, h, dated::REFUSED, &h.to_le_bytes())
                        } else {
                            panel_has_data[pid as usize] = true;
                            dated::encoded(0, 0, 0, pid, h, dated::STOPPED, &h.to_le_bytes())
                        };
                        ref_model.apply(PANEL, &pkt);
                        push.offer_panel(session, pkt, Panels::One(pid), has_cells);
                    }
                } else {
                    let reset = next_u32() % 100 < 8;
                    if reset { follow_has_base.clear(); }
                    let same_hour = !reset && last_follow_hour >= 10 && next_u32() % 100 < 15;
                    let fh = if same_hour { last_follow_hour } else { step_hour.max(last_follow_hour + 1) };
                    last_follow_hour = fh;
                    let mut pkt = Vec::new();
                    crate::answer::begin(&mut pkt, 0, 0, fh, 0, reset);
                    for fid in [10u32, 11, CELL_1, CELL_2, CELL_3] {
                        if same_hour && follow_has_base.get(&fid).copied().unwrap_or(false) { continue; }
                        if next_u32() % 100 < 45 {
                            if !reset && !same_hour && next_u32() % 100 < 12 {
                                crate::answer::push(&mut pkt, fid, (fh - 2).max(10), &[]);
                            } else {
                                let use_changed = !reset && follow_has_base.get(&fid).copied().unwrap_or(false) && next_u32() % 100 < 40;
                                let answer_body = if use_changed {
                                    make_answer(true, &[(1, i64::from(fh) * 10 + i64::from(fid & 0xff))])
                                } else {
                                    make_answer(false, &[(1, i64::from(fh) * 10), (2, i64::from(fh) * 10 + 1)])
                                };
                                crate::answer::push_held(&mut pkt, fid, fh, &answer_body, if use_changed { 128 } else { 0 });
                                follow_has_base.insert(fid, true);
                            }
                        }
                    }
                    ref_model.apply(FOLLOW, &pkt);
                    push.offer_follow(session, pkt);
                }
            }

            push.credit(session, total_sent);
            let mut drain_guard = 0;
            loop {
                match push.try_take() {
                    Some((kind, bytes, seq, _)) => {
                        if kind == FOLLOW && !bytes.starts_with(&crate::answer::MAGIC.to_le_bytes()) {
                            continue;
                        }
                        if seq < last_seq_taken { trial_overtakes += 1; }
                        last_seq_taken = last_seq_taken.max(seq);
                        total_sent += crate::answer::charge(&bytes) as u64;
                        q_model.apply(kind, &bytes);
                        push.credit(session, total_sent);
                    }
                    None => {
                        if push.in_flight() == 0 { break; }
                        push.credit(session, total_sent);
                    }
                }
                drain_guard += 1;
                assert!(drain_guard < 500, "{ctx}: queue failed to drain");
            }
            assert_eq!((push.in_flight(), budget.used()), (0, 0), "{ctx}: charges balance");
            if push.counters().dropped > 0 {
                continue;
            }
            total_overtakes += trial_overtakes;
            checked_trials += 1;

            if !per_cell && !ref_model.ever_overflowed {
                assert!(!q_model.ever_overflowed, "{ctx}: queued delivery overflowed early_limit when serial delivery never did");
            }
            let max_h = step_hour.max(panel_hour[1]).max(panel_hour[2]).max(panel_hour[3]) + 5;
            for pid in [1u32, 2, 3] {
                if per_cell || !ref_model.ever_overflowed {
                    assert_eq!(q_model.panels[&pid].failed, ref_model.panels[&pid].failed, "{ctx}: panel {pid} failure parity");
                }
                if ref_model.panels[&pid].failed || q_model.panels[&pid].failed { continue; }
                for h in 10..=max_h {
                    assert_eq!(q_model.covering_panel(pid, h), ref_model.covering_panel(pid, h), "{ctx}: panel {pid} exact coverage mismatch at hour {h} (head {max_head})");
                }
            }
            for fid in [10u32, 11] {
                assert_eq!(q_model.followed[&fid].failed, ref_model.followed[&fid].failed, "{ctx}: follow {fid} failure parity");
                if ref_model.followed[&fid].failed { continue; }
                for h in 10..=max_h {
                    assert_eq!(q_model.covering_follow(fid, h), ref_model.covering_follow(fid, h), "{ctx}: follow {fid} exact coverage mismatch at hour {h} (head {max_head})");
                }
            }
        }
    }
    assert!(checked_trials > 500 && total_overtakes > 0, "property test exercised commute overtakes ({total_overtakes}) across {checked_trials} exact-delivery trials");
}

#[test]
fn an_answer_is_charged_at_the_size_its_lane_holds() {
    use crate::push::Push;
    let push = Push::with_budget(playhead::ByteBudget::new(1 << 20));
    let changed = routed(0, 10, 1, 4096);
    push.offer(changed.clone());
    push.try_take().unwrap();
    assert_eq!(push.in_flight(), 24 + 16 + 2 * 4096, "a changed answer is charged at the whole table it lands on");
    assert_eq!(crate::answer::charge(&changed), 24 + 16 + 2 * 4096, "the lane's copy and the page's one copy");
    let mut table = crate::answer::Table::default();
    table.clear(1, false);
    for key in [1u128, 5, 9] { table.row(key); table.cell(key as i64, State::Value); table.name("TAG"); }
    let mut body = Vec::new();
    table.encode(&mut body);
    assert_eq!(table.encoded_len(), body.len(), "the held size is the whole table's encoded size");
}

#[test]
fn reached_answers_drop_past_a_full_budget_without_credit_and_stay_bounded() {
    use crate::push::Push;
    let first = rows_message(10, false, &[(1, 1)]);
    let one = crate::answer::charge(&first);
    let budget = playhead::ByteBudget::new(one * 2);
    let push = Push::with_budget(budget.clone());
    push.offer(first);
    assert!(push.try_take().is_some(), "the first answer goes out; the page never credits it");
    for hour in 11..200 {
        push.head(0, 0, hour);
        push.offer(rows_message(hour, false, &[(1, i64::from(hour))]));
        while push.try_take().is_some() {}
        assert!(budget.used() <= one * 4, "hour {hour}: {} charged under a budget of {}", budget.used(), one * 2);
    }
    assert!(push.in_flight() <= one * 2, "{} in flight under a budget of {}: reached answers kept leaving without credit", push.in_flight(), one * 2);
    assert!(push.counters().dropped > 0, "stalled route dropped queued records past the budget");
    assert!(push.failed().is_none());
}

#[test]
fn under_pressure_a_shown_replacement_is_itself_replaced_once_the_picture_before_it_is_released() {
    use crate::push::Push;
    let first = rows_message(10, false, &[(1, 1)]);
    let one = crate::answer::charge(&first);
    let push = Push::with_budget(playhead::ByteBudget::new(one * 2));
    let session = push.session();
    push.offer(first);
    assert!(push.try_take().is_some(), "the first picture goes out");
    push.head(0, 0, 11);
    push.offer(rows_message(11, false, &[(1, 2)]));
    assert!(push.try_take().is_some(), "its replacement goes out past credit while the budget is full");
    push.credit(session, one as u64);
    push.head(0, 0, 12);
    push.offer(rows_message(12, false, &[(1, 3)]));
    assert!(push.try_take().is_some(), "once the first is released, the shown replacement can be replaced too");
}

#[test]
fn under_pressure_a_pinned_picture_never_blocks_the_replacement_of_another() {
    use crate::push::Push;
    let pinned = rows_message(9, false, &[(1, 1)]);
    let one = crate::answer::charge(&pinned);
    let push = Push::with_budget(playhead::ByteBudget::new(one * 3));
    let session = push.session();
    push.head(0, 0, 10);
    push.offer(pinned);
    assert!(push.try_take().is_some(), "a picture the page will keep pinned");
    push.offer(rows_message(10, false, &[(1, 2)]));
    assert!(push.try_take().is_some(), "a reached picture while the budget has room");
    push.head(0, 0, 11);
    push.offer(rows_message(11, false, &[(1, 3)]));
    assert!(push.try_take().is_some(), "its replacement goes out past credit while the budget is full");
    push.credit(session, one as u64);
    push.head(0, 0, 12);
    push.offer(rows_message(12, false, &[(1, 4)]));
    assert!(push.try_take().is_some(), "the page released the replaced picture, so the next replacement goes out though the pinned one is still held");
}

#[test]
fn queue_size_bound_keeps_packets_within_budget_and_drops_overflow_ahead_of_clock() {
    use crate::push::Push;
    let packet = rows_message(20, false, &[(1, 1)]);
    let one = crate::answer::charge(&packet);
    let budget = playhead::ByteBudget::new(one * 2);
    let push = Push::with_budget(budget.clone());
    push.head(0, 0, 0);
    push.offer(rows_message(20, false, &[(1, 1)]));
    push.offer(rows_message(30, false, &[(1, 2)]));
    assert_eq!(push.counters().dropped, 0, "packets within the route queue budget stay queued");
    push.offer(rows_message(40, false, &[(1, 3)]));
    assert_eq!(push.counters().dropped, 3, "the packet that would exceed the route queue budget drops the route backlog");
    assert_eq!(budget.used(), 0);
    assert!(push.failed().is_none());
    assert_eq!(push.counters().unwritten, 0);
}

#[test]
fn a_follow_names_lanes_its_id_belongs_to() {
    let registry = crate::registry::Registry::new(None);
    let ask = |params: serde_json::Value| registry.answer(std::path::Path::new("x"), "follow", &params, |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel")));
    let wrong = ask(serde_json::json!({ "id": 7, "branchId": "1", "clock": 0, "lane": 0, "lanes": 3 }));
    assert!(wrong.unwrap_err().to_string().contains("not of lane 0 among 3"));
    let right = ask(serde_json::json!({ "id": 7, "branchId": "1", "clock": 0, "lane": 1, "lanes": 3 })).unwrap();
    assert_eq!(right["available"], false, "no playhead here, but the lane class is accepted");
}

fn at_seek(route: u32, seek: u32, hour: Hour, value: i64) -> Vec<u8> {
    let mut message = Vec::new();
    crate::answer::begin(&mut message, route, 0, hour, seek, false);
    crate::answer::push(&mut message, 7, hour, &body(7, value));
    message
}

#[test]
fn review_7_e_native_work_is_bound_to_its_session_and_seek() {
    use crate::push::{FOLLOW, Panels, Push};
    let registry = crate::registry::Registry::new(None);
    registry.restart("new page");
    let ask = |session: &str| registry.answer(std::path::Path::new("x"), "follow", &serde_json::json!({ "id": 7, "branchId": "1", "clock": 0, "session": session }),
        |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel"))).unwrap();
    assert_eq!(ask("old page")["reason"], "This page was replaced by a newer one; its follows ended with it", "a follow of the retired page changes nothing");
    assert_ne!(ask("new page")["reason"], ask("old page")["reason"], "the current page's follow goes on");

    let push = Push::default();
    push.head(0, 0, 1000);
    push.offer(at_seek(0, 1, 100, 1));
    push.offer(at_seek(0, 1, 124, 2));
    let (_, first, ..) = push.try_take().expect("the rewind's first answer");
    assert_eq!(crate::answer::decode(&first).unwrap().hour, 100, "a head of the old seek never merges away the new seek's destination answer");

    let push = Push::default();
    let session = push.session();
    push.offer_panel(session, crate::panel::dated::encoded(0, 0, 1, 3, 50, crate::panel::dated::STANDS, &[]), Panels::One(3), false);
    push.offer(at_seek(0, 2, 10, 3));
    let (kind, ..) = push.try_take().expect("the new seek's answer");
    assert!(kind == FOLLOW && push.try_take().is_none(), "an older seek's panel packet is retired with the follow answers of that seek");

    let small = at_seek(0, 0, 5, 1);
    let push = Push::with_budget(playhead::ByteBudget::new(crate::answer::charge(&small) * 2));
    let mut huge = Vec::new();
    crate::answer::begin(&mut huge, 0, 0, 6, 0, false);
    crate::answer::push_held(&mut huge, 7, 6, &body(7, 1), 1 << 20);
    push.offer(huge);
    assert!(push.failed().is_none() && push.try_take().is_some(), "past the budget the answer is still sent");
    push.restart();
    push.offer(small);
    assert!(push.failed().is_none() && push.try_take().is_some(), "a new page plays, with its drain still taking messages");
}

#[test]
fn review_8_one_session_token_binds_every_page_command_and_every_packet() {
    use crate::push::Push;
    let held = paper();
    let registry = crate::registry::Registry::new(Some(Arc::new(Papers(BTreeMap::from([("a".to_owned(), Arc::clone(&held))])))));
    let path = std::path::Path::new("a");
    registry.restart("page 1");
    let mut asked = follow_request(1, "country-measure", "politicalPower");
    asked["session"] = "page 1".into();
    assert_eq!(registry.answer(path, "follow", &asked, |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel"))).unwrap()["available"], true);
    registry.restart("page 2");
    let clock = |session: &str| registry.answer(path, "follow-clock", &serde_json::json!({ "branchId": "1", "clock": 0, "frameRate": 60.0, "seek": 1, "hour": 5.0, "speed": 1.0, "session": session }),
        |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel"))).unwrap();
    asked["session"] = "page 2".into();
    asked["id"] = 2.into();
    registry.answer(path, "follow", &asked, |_| Ok(nobody()), Box::new(|_| unreachable!("these requests carry no panel"))).unwrap();
    assert_eq!(clock("page 1")["reason"], "This page was replaced by a newer one; its follows ended with it", "a retired page's clock never anchors the new page's host");
    assert!(held.anchors.lock().unwrap().is_empty());
    assert_eq!(clock("page 2")["available"], true);

    let push = Push::default();
    let old = push.session();
    push.restart();
    push.offer_follow(old, at_seek(0, 0, 5, 1));
    assert!(push.try_take().is_none(), "work a retired session started is never sent to the new page");
    push.offer(at_seek(0, 2, 10, 2));
    assert!(push.try_take().is_some());
    push.offer(at_seek(0, 1, 11, 3));
    assert!(push.try_take().is_none(), "an older seek stays retired after the newer seek's packet has drained");

    let pages = record(quiet);
    let mut harness = harness(&pages);
    harness.follow.serve(4);
    harness.follow.follow(6, keys(&[100], &[1])).unwrap();
    harness.seek(0);
    let played = harness.play(48);
    assert!(!played.is_empty() && played.iter().all(|played| played.stats.session == 4), "each step names the session its subscriptions were registered in");
}

#[test]
fn naval_losses_evaluates_running_totals_and_refuses_when_sentinel_unknown() {
    use replay_data::rows::NavalLossTotalRow as Row;
    let table = TableId::NavalLossTotals.index();
    let kinds: Vec<Vec<Kind>> = replay_recording::schema::schema().unwrap().into_iter().take(table + 1).collect();
    let number = |kind: Kind, value: i64| match kind {
        Kind::I64 => Cell::I64(value),
        Kind::I32 => Cell::I32(value as i32),
        Kind::U32 => Cell::U32(value as u32),
        Kind::U8 => Cell::U8(value as u8),
        Kind::Bool => Cell::Bool(value != 0),
        Kind::OptionalI32 => Cell::OptionalI32(Some(value as i32)),
        Kind::OptionalI64 => Cell::OptionalI64(Some(value)),
        other => panic!("naval loss totals hold no {other:?} column"),
    };
    let ger = Key::Holder.of(1, 9001);
    let ita = Key::Holder.of(2, 9002);
    let rows = [
        (Row::run_key(Row::SHIPS), 0, 1),
        (Row::run_key(Row::CONVOYS), 0, 1),
        (Row::holder_key(ger, Row::LOST, Row::SHIPS), 3, 0),
        (Row::holder_key(ger, Row::LOST, Row::CONVOYS), 5, 0),
        (Row::holder_key(ita, Row::LOST, Row::SHIPS), 2, 0),
        (Row::holder_key(ita, Row::SUNK_BY, Row::SHIPS), 40, 0),
    ];
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing { cap: 256, ..Writing::at(3) }).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for hour in 0..4 {
        store.begin_hour(hour, &[]).unwrap();
        for held in 0..=table {
            store.status(held, Status::Complete, true, Some(hour), true).unwrap();
        }
        let mut changes = store.table(table).unwrap();
        let columns = &kinds[table];
        if hour == 0 {
            for (slot, &(key, total, counted_from)) in rows.iter().enumerate() {
                let slot = slot as u32;
                changes.insert_at(slot, key).unwrap();
                changes.write(slot, 0, number(columns[0], total)).unwrap();
                changes.write(slot, 2, number(columns[2], counted_from)).unwrap();
                changes.write(slot, 3, number(columns[3], 0)).unwrap();
            }
        } else if hour == 2 {
            changes.write(0, 3, number(columns[3], 1)).unwrap();
            changes.write(1, 3, number(columns[3], 1)).unwrap();
        }
        store.end_hour(&mut memory).unwrap();
    }
    store.end_run(&mut memory).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds });
    let mut holders = Holders::default();
    let held = |slot, id| vec![HolderRun { first: 0, last: 3, holder: Holder::Found { slot, id }, starts_segment: true }];
    holders.tags.insert("GER".into(), held(1, 9001));
    holders.tags.insert("ITA".into(), held(2, 9002));
    holders.tags.insert("JAP".into(), held(3, 9003));
    let timeline = Stretches { id: 1, first: 0, last: 3, stretches: vec![Stretch { run: 0, first: 0, last: 3, join: Join::Start }], unheld: Vec::new() };
    let identity = Identity { holders: Arc::new(holders), timeline: Arc::new(timeline) };
    let request = |countries: serde_json::Value, top: serde_json::Value, kind: &str| {
        crate::parse(&serde_json::json!({ "product": "naval-losses", "params": { "branchId": "1", "countries": countries, "top": top, "attribution": "lost", "kind": kind, "period": "running" } }), &identity).unwrap()
    };
    let top = request(serde_json::json!([]), serde_json::json!(10), "both");
    let chosen = request(serde_json::json!(["GER", "JAP"]), serde_json::Value::Null, "ships");
    let at = |query: &Query, hour: Hour| {
        let mut lane = Lane::new(Arc::clone(&pages), 0, table, &[0, 1, 2, 3, 4], false).unwrap();
        lane.seek(hour).unwrap();
        Answer::parse(&answer_at(query, &lane).unwrap()).unwrap()
    };
    let rows_of = |answer: &Answer| (0..answer.keys.len()).map(|row| (answer.values[row], answer.name(row).unwrap().to_owned())).collect::<Vec<_>>();
    assert!(at(&top, 1).not_recorded.is_some(), "losses before the counting is known are not recorded");
    assert!(at(&chosen, 1).not_recorded.is_some(), "losses before the counting is known are not recorded");
    assert_eq!(rows_of(&at(&top, 2)), [(8, "GER".to_owned()), (2, "ITA".to_owned())], "ships and convoys summed, sinkings by others left out, no loss no row");
    assert_eq!(rows_of(&at(&chosen, 2)), [(3, "GER".to_owned()), (0, "JAP".to_owned())], "a chosen country with no loss is a known zero");
}

static POWER_SLOT: Series = Series {
    kind: "test-power-slot",
    table: TableId::Countries,
    key: Key::Slot,
    parts: None,
    measures: &[Measure { name: "power", column: Column::Cell("political_power_raw"), decimals: 5, aggregates: 0, carry: Carry::None, rank: 0, rank_zero: dashboard_declare::RankZero::Keep }],
    drops: false,
    absent_none: false,
};

#[test]
fn a_followed_top_ranks_a_recorded_row_whose_holder_is_not_resolved_under_its_slot() {
    let pages = record(quiet);
    let timeline = Stretches { id: 1, first: 0, last: HOURS as Hour - 1, stretches: vec![Stretch { run: 0, first: 0, last: HOURS as Hour - 1, join: Join::Start }], unheld: Vec::new() };
    let mut holders = Holders::default();
    holders.tags.insert("AAA".into(), vec![HolderRun { first: 0, last: HOURS as Hour - 1, holder: Holder::Found { slot: 101, id: 999 }, starts_segment: true }]);
    let identity = Identity { holders: Arc::new(holders), timeline: Arc::new(timeline) };
    let top = Query::series_top(&POWER_SLOT, &POWER_SLOT.measures[0], 3, identity);
    let mut harness = harness(&pages);
    harness.follow.follow(5, top.clone()).unwrap();
    harness.seek(0);
    let played = harness.play(40);
    let (_, hour, answer) = played.iter().flat_map(Played::answers).filter(|(at, _, _)| *at == 5).next_back().expect("an answer");
    assert_eq!(answer, harness.fresh(&top, hour));
    assert_eq!(answer.not_recorded, None, "rows whose values are recorded never refuse the top");
    assert_eq!(answer.keys.len(), 3, "the top holds its rows whether or not their holders are named");
    assert_eq!((answer.keys[0], answer.name(0)), (101, Some("AAA")));
    let names: Vec<Option<&str>> = (0..3).map(|row| answer.name(row)).collect();
    for (row, key) in answer.keys.iter().enumerate() {
        let country = dashboard_query::identity::Country::Slot(u32::try_from(*key).unwrap());
        let wanted = if row == 0 { "AAA".to_owned() } else { country.key().into_owned() };
        assert_eq!(names[row], Some(wanted.as_str()), "row {row} is named by its tag or its slot");
        assert_eq!(answer.reason(row).map(str::to_owned), (row != 0).then(|| series_identity(&top).holders.unnamed(&country, hour)));
        assert_eq!(answer.states[row], State::Value);
    }
}

#[test]
fn a_followed_population_rank_of_packed_holders_returns_the_country_slot() {
    static PACKED: Series = Series { kind: "test-packed-power", key: Key::Holder, ..POWER };
    let pages = record(|at, changes| {
        if at == 0 {
            changes.insert_at(0, Key::Holder.of(9, 999)).unwrap();
            changes.write(0, 2, Cell::OptionalI64(Some(45))).unwrap();
        }
    });
    let timeline = Stretches { id: 1, first: 0, last: HOURS as Hour - 1, stretches: vec![Stretch { run: 0, first: 0, last: HOURS as Hour - 1, join: Join::Start }], unheld: Vec::new() };
    let mut holders = Holders::default();
    holders.tags.insert("AAA".into(), vec![HolderRun { first: 0, last: HOURS as Hour - 1, holder: Holder::Found { slot: 9, id: 999 }, starts_segment: true }]);
    let query = Query::series_top(&PACKED, &PACKED.measures[0], 1, Identity { holders: Arc::new(holders), timeline: Arc::new(timeline) });
    let mut harness = harness(&pages);
    harness.follow.follow(5, query.clone()).unwrap();
    harness.seek(0);
    let (_, hour, answer) = harness.play(40).iter().flat_map(Played::answers).next_back().expect("a ranked answer");
    assert_eq!(answer, harness.fresh(&query, hour));
    assert_eq!(answer.keys, [9]);
    assert_eq!((answer.values[0], answer.name(0)), (45, Some("AAA")));
}

#[test]
fn a_pending_product_on_a_paused_seek_delivers_its_anchored_hour_after_warming() {
    use crate::{
        host::PlayheadHost,
        panel::{AtHour, ExactReader, PanelRows, ProductRead, ProductReader, Wake, dated},
        push::{PANEL, Push},
        registry::{Anchor, Hosted},
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    struct NoRows;
    impl PanelRows for NoRows {
        fn reader(&self, _tables: &[TableId]) -> io::Result<Box<dyn crate::panel::PanelReader>> {
            Err(io::Error::other("unused"))
        }
    }

    let pages = record(quiet);
    let follow = Follow::new(0);
    let hook = follow.hook(Box::new(|_, _| {}));
    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 16, ring: 8, speed: 1.0, rate: 60.0, ..Settings::default() };
    let playhead = Arc::new(Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings, Some(hook)).unwrap());
    let anchored = |playhead: &Playhead, anchor: Anchor| {
        playhead.frame_rate(anchor.frame_rate);
        if anchor.sought {
            playhead.seek(anchor.hour);
        }
        if !anchor.playing {
            playhead.pause();
        }
    };
    let held = crate::host::held_tables(&*pages, pages.kinds.len());
    let host = PlayheadHost::new(Arc::clone(&playhead), |p| p, anchored, follow, held).unwrap();
    let push = Arc::new(Push::default());
    host.show_panels(Arc::new(NoRows), Arc::clone(&push));

    let ready = Arc::new(AtomicBool::new(false));
    let (wake_reader, wakes) = mpsc::channel::<Wake>();
    let ready_reader = Arc::clone(&ready);
    let advance: ProductReader = Arc::new(move |hour, _values, _cancelled, step| {
        if !ready_reader.load(Ordering::Acquire) {
            if let Some(wake) = step.and_then(|s| s.wake.cloned()) {
                wake_reader.send(wake).unwrap();
            }
            return ProductRead { value: Arc::new(Err(String::new())), rows: 0, fresh: false, pending: true, live: None, lanes: Vec::new(), from: Hour::MIN };
        }
        ProductRead { value: Arc::new(Ok(serde_json::json!({ "hour": hour }))), rows: 1, fresh: true, pending: false, live: None, lanes: Vec::new(), from: Hour::MIN }
    });
    let cold: ExactReader = Arc::new(|hour, _: &AtomicBool| Ok(serde_json::json!({ "hour": hour })));
    let panel = Box::new(AtHour { read: Arc::clone(&cold), cold, advance });
    host.prepare_panel(panel).unwrap()(8, 4).unwrap();
    host.anchor(Anchor { seek: 4, frame_rate: 60.0, hour: 50, position: 50.0, timestamps: None, playing: false, speed: 1.0, sought: true });

    let mut led_past = false;
    while !led_past {
        let (kind, bytes, _) = push.take_sent().expect("panel packet");
        if kind != PANEL || dated::stamp(&bytes).map(|(_, seek)| seek) != Some(4) {
            continue;
        }
        for r in dated::records(&bytes).unwrap() {
            if r.kind == dated::COMPLETED && r.hour > 50 {
                led_past = true;
            }
        }
    }
    let wake = wakes.recv().expect("wake callback captured while pending");
    ready.store(true, Ordering::Release);
    wake();

    let mut data_hours = Vec::new();
    while data_hours.is_empty() {
        let (kind, bytes, _) = push.take_sent().expect("panel packet after wake");
        if kind != PANEL || dated::stamp(&bytes).map(|(_, seek)| seek) != Some(4) {
            continue;
        }
        for r in dated::records(&bytes).unwrap() {
            if r.id == 8 && r.kind == dated::DATA {
                data_hours.push(r.hour);
            }
        }
    }
    assert!(data_hours.contains(&50), "anchored hour 50 must be delivered after pending warm finishes, got data at {data_hours:?}");
    drop(host);
}

#[test]
fn a_new_page_seek_while_paused_reads_again_a_product_whose_read_it_cancelled() {
    use crate::{
        host::PlayheadHost,
        panel::{AtHour, ExactReader, PanelRows, ProductRead, ProductReader, dated},
        push::{PANEL, Push},
        registry::{Anchor, Hosted},
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    struct NoRows;
    impl PanelRows for NoRows {
        fn reader(&self, _tables: &[TableId]) -> io::Result<Box<dyn crate::panel::PanelReader>> {
            Err(io::Error::other("unused"))
        }
    }

    let pages = record(quiet);
    let follow = Follow::new(0);
    let hook = follow.hook(Box::new(|_, _| {}));
    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 16, ring: 8, speed: 1.0, rate: 60.0, ..Settings::default() };
    let playhead = Arc::new(Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, Timeline::whole(&*pages).unwrap(), settings, Some(hook)).unwrap());
    let anchored = |playhead: &Playhead, anchor: Anchor| {
        playhead.frame_rate(anchor.frame_rate);
        if anchor.sought {
            playhead.seek(anchor.hour);
        }
        if !anchor.playing {
            playhead.pause();
        }
    };
    let held = crate::host::held_tables(&*pages, pages.kinds.len());
    let host = PlayheadHost::new(Arc::clone(&playhead), |p| p, anchored, follow, held).unwrap();
    let push = Arc::new(Push::default());
    host.show_panels(Arc::new(NoRows), Arc::clone(&push));

    let first = Arc::new(AtomicBool::new(true));
    let (started, wait_started) = std::sync::mpsc::channel();
    let advance: ProductReader = Arc::new(move |hour, _values, cancelled, _step| {
        if first.swap(false, Ordering::AcqRel) {
            started.send(()).unwrap();
            while !cancelled.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        }
        ProductRead { value: Arc::new(Ok(serde_json::json!({ "hour": hour }))), rows: 1, fresh: true, pending: false, live: None, lanes: Vec::new(), from: Hour::MIN }
    });
    let cold: ExactReader = Arc::new(|hour, _: &AtomicBool| Ok(serde_json::json!({ "hour": hour })));
    host.prepare_panel(Box::new(AtHour { read: Arc::clone(&cold), cold, advance })).unwrap()(8, 4).unwrap();
    wait_started.recv().unwrap();

    let hour: Hour = 0;
    let epoch = playhead.epoch();
    host.anchor(Anchor { seek: 2, frame_rate: 60.0, hour, position: f64::from(hour), timestamps: None, playing: false, speed: 1.0, sought: false });
    assert_ne!(playhead.epoch(), epoch, "a new page seek is a seek, so every product is read again at it");

    loop {
        let (kind, bytes, _) = push.take_sent().expect("panel packet");
        if kind != PANEL || dated::stamp(&bytes).map(|(_, seek)| seek) != Some(2) {
            continue;
        }
        if dated::records(&bytes).unwrap().iter().any(|r| r.id == 8 && r.kind == dated::DATA && r.hour == hour) {
            break;
        }
    }
    drop(host);
}

#[test]
fn one_blocked_seed_leaves_other_panels_and_the_clock_free() {
    use crate::{
        host::PlayheadHost,
        panel::{AtHour, ExactReader, PanelRows, ProductRead, ProductReader, dated},
        push::{PANEL, Push},
        registry::{Anchor, Hosted},
    };
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

    struct NoRows;
    impl PanelRows for NoRows {
        fn reader(&self, _tables: &[TableId]) -> io::Result<Box<dyn crate::panel::PanelReader>> {
            unreachable!("exact panels supply their own rows")
        }
    }
    fn product(advance: ProductReader) -> Box<AtHour> {
        let cold: ExactReader = Arc::new(|_, _| unreachable!("the incremental reader supplies the seed"));
        Box::new(AtHour { read: cold.clone(), cold, advance })
    }
    let pages = record(quiet);
    let follow = Follow::new(0);
    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 16, ring: 8, speed: 1.0, rate: 60.0, ..Settings::default() };
    let gate_from = Arc::new(AtomicI32::new(i32::MAX));
    let threshold = gate_from.clone();
    let (arrived, clock_steps) = mpsc::channel();
    let mut hook = follow.hook(Box::new(|_, _| {}));
    let hook: playhead::Hook = Box::new(move |step, lanes| {
        if step.first() <= step.last() && step.first() >= threshold.load(Ordering::Acquire) {
            let (advance, permit) = mpsc::channel();
            if arrived.send((step.first(), step.last(), advance)).is_ok() { let _ = permit.recv(); }
        }
        hook(step, lanes);
    });
    let playhead = Arc::new(Playhead::new(pages.clone(), Timeline::whole(&*pages).unwrap(), settings, Some(hook)).unwrap());
    let host = PlayheadHost::new(playhead, |p| p, |p, a| { if a.sought { p.seek(a.hour); } }, follow, vec![true; TABLE + 1]).unwrap();
    let steps = clock_steps;
    let push = Arc::new(Push::default());
    host.show_panels(Arc::new(NoRows), push.clone());
    let reading = |hour| ProductRead { value: Arc::new(Ok(serde_json::json!({ "hour": hour }))), rows: 1, fresh: true, pending: false, live: None, lanes: Vec::new(), from: Hour::MIN };
    let first_data = |id| {
        host.prepare_panel(product(Arc::new(move |hour, _, _, _| reading(hour)))).unwrap()(id, 1).unwrap();
        loop {
            let (kind, bytes, _, offered) = push.take_kind().unwrap();
            if kind == PANEL {
                let records = dated::records(&bytes).unwrap();
                for r in &records {
                    assert!(r.id != 2 || r.kind == dated::WAITING, "the removed slow reader never publishes a value");
                    if r.id == id && r.kind != dated::WAITING {
                        assert_eq!(r.kind, dated::DATA);
                        assert!(!r.stepped(), "this subscription receives its seed before its steps");
                        assert!(records.iter().any(|next| next.id == id && next.kind == dated::WAITING && next.hour == r.hour + 1),
                            "a route completion cannot extend the seed over hours its reader has yet to visit");
                        return offered;
                    }
                }
            }
        }
    };
    first_data(1);
    host.unpanel(1);
    let (entered, started) = mpsc::channel();
    let (open, gate) = mpsc::channel();
    let (finished, done) = mpsc::channel();
    let gate = Mutex::new(gate);
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    host.prepare_panel(product(Arc::new(move |hour, _, cancelled: &AtomicBool, _| {
        counted.fetch_add(1, Ordering::Relaxed);
        entered.send(()).unwrap();
        gate.lock().unwrap().recv().unwrap();
        finished.send(cancelled.load(Ordering::Acquire)).unwrap();
        reading(hour)
    }))).unwrap()(2, 1).unwrap();
    started.recv().unwrap();
    first_data(3);
    host.anchor(Anchor { seek: 5, hour: 50, position: 50.0, playing: false, sought: true, frame_rate: 60.0, speed: 1.0, timestamps: None });
    host.unpanel(2);
    open.send(()).unwrap();
    assert!(done.recv().unwrap());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let anchored_at = first_data(4);
    host.unpanel(3);
    host.unpanel(4);

    let (started, seed_hour) = mpsc::channel();
    let (resume, gate) = mpsc::channel();
    let gate = Mutex::new(gate);
    let (reader, catching_up) = mpsc::channel();
    let first = AtomicBool::new(true);
    let worker = Arc::new(Mutex::new(None));
    let reading_on = worker.clone();
    host.prepare_panel(product(Arc::new(move |hour, _, _, step| {
        if step.is_none() { *reading_on.lock().unwrap() = Some(std::thread::current().id()); }
        if first.swap(false, Ordering::Relaxed) {
            started.send(hour).unwrap();
            gate.lock().unwrap().recv().unwrap();
        } else {
            let _ = reader.send((hour, step.is_some(), std::thread::current().id()));
        }
        reading(hour)
    }))).unwrap()(6, 1).unwrap();
    let start = seed_hour.recv().unwrap();
    let quantum = 6;
    gate_from.store(start + 3 * quantum, Ordering::Release);
    host.playhead().pace(60.0 * f64::from(quantum));
    host.playhead().anchor(f64::from(start + 6 * quantum), true, 0.0, anchored_at, false);
    let (_, through, advance) = steps.recv().unwrap();
    let (returned, handoffs) = mpsc::channel();
    let _watch = host.playhead().watch_control(Arc::new(move || {
        if worker.lock().unwrap().take_if(|id| *id == std::thread::current().id()).is_some() { let _ = returned.send(()); }
    }));
    resume.send(()).unwrap();
    handoffs.recv().unwrap();
    host.playhead().anchor(f64::from(through + quantum), true, 0.0, anchored_at, false);
    advance.send(()).unwrap();
    let (next, last, advance) = steps.recv().unwrap();
    handoffs.recv().unwrap();
    let cold: Vec<_> = catching_up.try_iter().collect();
    assert_eq!(cold.first().map(|r| r.0), Some(start + quantum));
    assert_eq!(cold.last().map(|r| r.0), Some(through), "catch-up covers the scheduling step");
    assert!(cold.iter().all(|r| !r.1));
    assert_eq!(next, through + 1);
    gate_from.store(i32::MAX, Ordering::Release);
    advance.send(()).unwrap();
    let (hour, stepped, hook) = catching_up.recv().unwrap();
    assert!(stepped, "the next playing step rejoins the product");
    assert_eq!(hour, last);
    assert_ne!(cold[0].2, hook, "the playhead keeps stepping on a different thread");

    host.unpanel(6);
    let (control_moved, control_moves) = mpsc::channel();
    let _control_watch = host.playhead().watch_control(Arc::new(move || { let _ = control_moved.send(()); }));
    while host.playhead().holding().is_some() {
        control_moves.recv().unwrap();
    }
    let (s7_entered_tx, s7_entered_rx) = mpsc::channel();
    let (s7_release_tx, s7_release_rx) = mpsc::channel();
    let s7_release_rx = Mutex::new(s7_release_rx);
    let s7_first = AtomicBool::new(true);
    let seeding = Arc::new(Mutex::new(None));
    let seeding_on = seeding.clone();
    host.prepare_panel(product(Arc::new(move |hour, _, _, step| {
        if s7_first.swap(false, Ordering::Relaxed) {
            *seeding_on.lock().unwrap() = Some(std::thread::current().id());
            let _ = s7_entered_tx.send(hour);
            let _ = s7_release_rx.lock().unwrap().recv();
            return reading(hour);
        }
        let val = if step.is_none() { 9_007_199_254_740_990 + i64::from(hour) } else { i64::from(hour) };
        let cells = crate::live::Cells::default();
        cells.value("r", "v", crate::live::Source { table: TableId::ALL[TABLE], column: 0, key: 100 }, val as f64);
        let live = Arc::new(cells.finish());
        let mut group = serde_json::json!({ "table": TABLE, "columns": [0], "keys": ["100"], "values": [val] });
        if val.unsigned_abs() >= 1 << 53 {
            group["rawValues"] = serde_json::json!([val.to_string()]);
        }
        let value = Arc::new(Ok(serde_json::json!({
            "live": {
                "groups": [group],
                "rows": ["r"],
                "kinds": [{ "id": "v", "op": "value", "scale": 1.0 }],
                "cells": [0, 0, 1, 0, 0, 0],
            }
        })));
        ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
    }))).unwrap()(7, 1).unwrap();
    let start7 = s7_entered_rx.recv().unwrap();
    gate_from.store(start7 + 3 * quantum, Ordering::Release);
    host.playhead().anchor(f64::from(start7 + 3 * quantum), true, 0.0, anchored_at, false);
    let (_, _, advance7) = steps.recv().unwrap();
    let (put_back, seeded) = mpsc::channel();
    let _seed_watch = host.playhead().watch_control(Arc::new(move || {
        if seeding.lock().unwrap().take_if(|id| *id == std::thread::current().id()).is_some() { let _ = put_back.send(()); }
    }));
    s7_release_tx.send(()).unwrap();
    seeded.recv().unwrap();
    advance7.send(()).unwrap();
    let (caught, wide7) = 'wide: loop {
        let (kind, bytes, session) = push.take_sent().expect("six-hour wide catch-up packet");
        push.credit(session, u64::MAX);
        if kind != PANEL { continue; }
        for r in dated::records(&bytes).unwrap() {
            if r.id == 7 && r.hour > start7 && r.kind == dated::DATA {
                break 'wide (r.hour, rmp_serde::from_slice::<serde_json::Value>(r.body).expect("panel 7 wide catch-up data"));
            }
        }
    };
    assert!(caught >= start7 + quantum, "the first reading past the seed comes from the catch-up");
    let expected_wide = (9_007_199_254_740_990 + i64::from(caught)).to_string();
    assert_eq!(wide7["live"]["groups"][0]["rawValues"], serde_json::json!([expected_wide]), "multi-hour (quantum=6) catch-up preserves lossless wide rawValues");
    assert!(wide7["live"]["groups"][0].get("follow").is_none());
    gate_from.store(i32::MAX, Ordering::Release);
    drop(steps);

    host.unpanel(7);
    let (s8_entered_tx, s8_entered_rx) = mpsc::channel();
    let (s8_release_tx, s8_release_rx) = mpsc::channel();
    let s8_release_rx = Mutex::new(s8_release_rx);
    let s8_first = AtomicBool::new(true);
    let s8_reads = Arc::new(Mutex::new(Vec::new()));
    let s8_reads_cb = Arc::clone(&s8_reads);
    let proven: Arc<crate::panel::PanelData> = Arc::new(Ok(serde_json::json!({ "tile": "closed before the clock" })));
    let s8_start = Arc::new(AtomicI32::new(i32::MAX));
    let s8_start_cb = Arc::clone(&s8_start);
    host.prepare_panel(product(Arc::new(move |hour, seed, _, step| {
        if s8_first.swap(false, Ordering::Relaxed) {
            s8_start_cb.store(hour, Ordering::Release);
            let _ = s8_entered_tx.send(hour);
            let _ = s8_release_rx.lock().unwrap().recv();
        } else if step.is_none() {
            s8_reads_cb.lock().unwrap().push((hour, seed));
        }
        let read_again = hour == s8_start_cb.load(Ordering::Acquire).saturating_add(2 * quantum);
        ProductRead { value: Arc::clone(&proven), rows: 1, fresh: read_again, pending: false, live: None, lanes: Vec::new(), from: Hour::MIN }
    }))).unwrap()(8, 1).unwrap();
    let start8 = s8_entered_rx.recv().unwrap();
    host.playhead().anchor(f64::from(start8 + 3 * quantum), true, 0.0, anchored_at, false);
    host.playhead().wait_newest(start8 + 2 * quantum).unwrap();
    s8_release_tx.send(()).unwrap();
    let mut caught = Vec::new();
    while !caught.iter().any(|&(hour, _)| hour == start8 + 2 * quantum) {
        let (kind, bytes, session) = push.take_sent().expect("a catch-up packet for panel 8");
        push.credit(session, u64::MAX);
        if kind != PANEL { continue; }
        caught.extend(dated::records(&bytes).unwrap().into_iter().filter(|r| r.id == 8 && !r.stepped() && r.kind != dated::WAITING && [start8 + quantum, start8 + 2 * quantum].contains(&r.hour)).map(|r| (r.hour, r.kind)));
    }
    let reads = s8_reads.lock().unwrap().clone();
    assert!(!reads.is_empty() && reads.iter().all(|&(_, seed)| !seed), "multi-hour catch-up reads through the held interval, never as a seed: {reads:?}");
    assert_eq!(
        caught,
        [(start8 + quantum, dated::STANDS), (start8 + 2 * quantum, dated::AGAIN)],
        "a value the reader carried across the skipped hours stands on the previous record; one it read again at the hour proves nothing about the hours between and starts again, both without the body"
    );
}

type StepGate = mpsc::Receiver<(Hour, mpsc::Sender<()>)>;

fn playing_panel(advance: crate::panel::ProductReader) -> (Arc<crate::host::PlayheadHost<Playhead>>, Arc<crate::push::Push>, Arc<std::sync::atomic::AtomicI32>, StepGate) {
    use crate::{panel::{AtHour, ExactReader, PanelRows}, registry::Hosted};
    use std::sync::atomic::{AtomicI32, Ordering};
    struct NoRows;
    impl PanelRows for NoRows {
        fn reader(&self, _tables: &[TableId]) -> io::Result<Box<dyn crate::panel::PanelReader>> {
            unreachable!("exact panels supply their own rows")
        }
    }
    let pages = record(quiet);
    let follow = Follow::new(0);
    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 16, ring: 8, speed: 1.0, rate: 60.0, ..Settings::default() };
    let gate_from = Arc::new(AtomicI32::new(i32::MAX));
    let (threshold, (arrived, gated)) = (gate_from.clone(), mpsc::channel());
    let mut inner = follow.hook(Box::new(|_, _| {}));
    let hook: playhead::Hook = Box::new(move |step, lanes| {
        if step.first() <= step.last() && step.first() >= threshold.load(Ordering::Acquire) {
            let (permit, wait) = mpsc::channel();
            if arrived.send((step.last(), permit)).is_ok() { let _ = wait.recv(); }
        }
        inner(step, lanes);
    });
    let playhead = Arc::new(Playhead::new(pages.clone(), Timeline::whole(&*pages).unwrap(), settings, Some(hook)).unwrap());
    let host = crate::host::PlayheadHost::new(playhead, |p| p, |p, a| { if a.sought { p.seek(a.hour); } }, follow, vec![true; TABLE + 1]).unwrap();
    let push = Arc::new(crate::push::Push::default());
    host.show_panels(Arc::new(NoRows), push.clone());
    host.anchor(crate::registry::Anchor { seek: 1, hour: 0, position: 0.0, playing: false, sought: true, frame_rate: 60.0, speed: 360.0, timestamps: None });
    let cold: ExactReader = Arc::new(|_, _| unreachable!("the product reader supplies the seed"));
    host.prepare_panel(Box::new(AtHour { read: cold.clone(), cold, advance })).unwrap()(1, 1).unwrap();
    let (_, _, _, offered) = push.take_kind().expect("the seed's first record");
    host.playhead().pace(360.0);
    host.playhead().anchor(0.0, true, 360.0, offered, false);
    (host, push, gate_from, gated)
}

fn completed_batch(push: &crate::push::Push) -> (Hour, Vec<(u32, u8, Hour, bool)>) {
    use crate::panel::dated;
    loop {
        let (kind, bytes, session) = push.take_sent().expect("panel batches while playing");
        push.credit(session, u64::MAX);
        if kind != crate::push::PANEL { continue; }
        let records = dated::records(&bytes).unwrap();
        if let Some(completed) = records.iter().find(|r| r.kind == dated::COMPLETED).map(|r| r.hour) {
            return (completed, records.iter().filter(|r| r.kind != dated::COMPLETED).map(|r| (r.id, r.kind, r.hour, r.stepped())).collect());
        }
    }
}

fn joined(push: &crate::push::Push) -> Hour {
    loop {
        let (completed, records) = completed_batch(push);
        if records.iter().any(|&(id, kind, _, stepped)| id == 1 && stepped && kind != crate::panel::dated::WAITING) { return completed; }
    }
}

fn steady(proven_from_read: bool) -> crate::panel::ProductReader {
    Arc::new(move |hour, _, _, _| crate::panel::ProductRead {
        value: Arc::new(Ok(serde_json::json!({ "steady": true }))), rows: 1, fresh: true, pending: false, live: None, lanes: Vec::new(),
        from: if proven_from_read { hour } else { Hour::MIN },
    })
}

#[test]
fn a_step_never_completes_a_panel_whose_product_it_did_not_read() {
    use std::sync::atomic::Ordering;
    let (host, push, gate_from, gated) = playing_panel(steady(false));
    gate_from.store(joined(&push).saturating_add(1), Ordering::Release);
    let (blocked, permit) = gated.recv().unwrap();
    let held = std::iter::repeat_with(|| host.hold_product(1)).find_map(|hold| hold).unwrap();
    gate_from.store(i32::MAX, Ordering::Release);
    permit.send(()).unwrap();
    let records = loop {
        let (completed, records) = completed_batch(&push);
        if completed >= blocked { break records; }
    };
    assert!(records.iter().any(|&(id, kind, ..)| id == 1 && kind == crate::panel::dated::WAITING),
        "the step through {blocked} completed while panel 1's product was busy and unread: its last value would be claimed for those hours");
    held();
}

#[test]
fn a_step_restates_a_reading_its_product_proves_only_from_the_step_end() {
    use crate::panel::dated;
    let (_host, push, ..) = playing_panel(steady(true));
    joined(&push);
    let mut restated = 0;
    for _ in 0..4 {
        let (completed, records) = completed_batch(&push);
        let own: Vec<_> = records.iter().filter(|r| r.0 == 1).map(|&(_, kind, hour, _)| (kind, hour)).collect();
        assert!(own.last().is_some_and(|&(_, hour)| hour == completed) && own.iter().all(|&(kind, _)| kind == dated::AGAIN || kind == dated::DATA),
            "a value proven only at {completed} starts again there instead of standing over the step's unread hours: {own:?}");
        restated += own.iter().filter(|&&(kind, _)| kind == dated::AGAIN).count();
    }
    assert!(restated > 0, "an equal value restates itself without its body");
    let (_host, push, ..) = playing_panel(steady(false));
    joined(&push);
    let spoken: Vec<_> = (0..4).map(|_| completed_batch(&push).1.into_iter().filter(|r| r.0 == 1).collect::<Vec<_>>()).filter(|own| !own.is_empty()).collect();
    assert!(spoken.len() <= 1 && spoken.iter().flatten().all(|r| r.1 == dated::DATA), "a value proven across the step stays silent after its one resend: {spoken:?}");
}

#[test]
fn a_followed_top_of_chosen_countries_is_not_recorded_where_their_holders_are_not_known() {
    let pages = record(quiet);
    let timeline = Stretches { id: 1, first: 0, last: HOURS as Hour - 1, stretches: vec![Stretch { run: 0, first: 0, last: HOURS as Hour - 1, join: Join::Start }], unheld: Vec::new() };
    let mut holders = Holders::default();
    holders.tags.insert("AAA".into(), vec![HolderRun { first: 0, last: 9, holder: Holder::Found { slot: 103, id: 103 }, starts_segment: true }]);
    let reason = "Countries was not complete at these hours";
    holders.unknown.push((10, HOURS as Hour - 1, reason.to_owned()));
    let identity = Identity { holders: Arc::new(holders), timeline: Arc::new(timeline) };
    let chosen = Query::Series {
        series: &POWER_SLOT,
        measure: &POWER_SLOT.measures[0],
        countries: vec![dashboard_query::identity::Country::from("AAA")],
        mode: SeriesMode::Top { n: 3, least: false, population: false },
        identity,
    };
    let mut harness = harness(&pages);
    harness.follow.follow(5, chosen.clone()).unwrap();
    harness.seek(0);
    let played = harness.play(40);
    let (_, hour, answer) = played.iter().flat_map(Played::answers).filter(|(at, _, _)| *at == 5).next_back().expect("an answer");
    assert!(hour >= 10, "the last answer falls in the unknown hours");
    let wanted = format!("Who holds each country is not known at this hour, so they cannot be ranked: {reason}");
    assert_eq!(answer.not_recorded.as_deref(), Some(wanted.as_str()), "never a successful empty ranking");
    assert_eq!(answer, harness.fresh(&chosen, hour), "the incremental ranking and the answer at the hour agree");
}

mod paused_reset {
    use super::*;
    use crate::{
        host::PlayheadHost,
        live::{CellValue, Cells, GroupState, Source},
        panel::{AtHour, ExactReader, PanelRows, ProductRead, ProductReader, dated},
        push::{PANEL, Push},
        registry::{Anchor, Hosted},
    };

    struct NoRows;
    impl PanelRows for NoRows {
        fn reader(&self, _tables: &[TableId]) -> io::Result<Box<dyn crate::panel::PanelReader>> {
            Err(io::Error::other("unused"))
        }
    }

    struct SinkGate {
        state: Mutex<(Option<Hour>, bool, bool)>,
        cv: std::sync::Condvar,
    }

    impl SinkGate {
        fn new() -> Arc<Self> {
            Arc::new(Self { state: Mutex::new((None, false, false)), cv: std::sync::Condvar::new() })
        }
        fn arm(&self, hour: Hour) {
            *self.state.lock().unwrap() = (Some(hour), false, false);
        }
        fn hit(&self, hour: Hour) {
            let mut s = self.state.lock().unwrap();
            if s.0 == Some(hour) && !s.2 {
                s.1 = true;
                self.cv.notify_all();
                while !s.2 {
                    s = self.cv.wait(s).unwrap();
                }
                s.0 = None;
            }
        }
        fn wait_entered(&self) {
            let mut s = self.state.lock().unwrap();
            while !s.1 {
                s = self.cv.wait(s).unwrap();
            }
        }
        fn release(&self) {
            let mut s = self.state.lock().unwrap();
            s.2 = true;
            self.cv.notify_all();
        }
    }

    fn product(advance: ProductReader) -> Box<AtHour> {
        let cold: ExactReader = Arc::new(|_, _| unreachable!("the incremental reader supplies the seed"));
        Box::new(AtHour { read: cold.clone(), cold, advance })
    }

    fn live_cell(column: usize, key: u128, value: i64) -> (Arc<crate::live::Live>, Arc<crate::panel::PanelData>) {
        let cells = Cells::default();
        cells.value("r", "v", Source { table: TableId::ALL[TABLE], column, key }, 1.0);
        let live = Arc::new(cells.finish());
        let json = serde_json::json!({ "live": live.json(&[GroupState::Values(vec![CellValue::Number(value)])]) });
        (live, Arc::new(Ok(json)))
    }

    fn take_panel_data(
        seen: &mut Vec<(u32, Hour, u8, Option<serde_json::Value>)>,
        push: &Push,
        target_id: u32,
        target_hour: Hour,
    ) -> serde_json::Value {
        take_panel_data_at_epoch(seen, push, None, target_id, target_hour)
    }

    fn take_panel_data_at_epoch(
        seen: &mut Vec<(u32, Hour, u8, Option<serde_json::Value>)>,
        push: &Push,
        expected_epoch: Option<(&Playhead, u64)>,
        target_id: u32,
        target_hour: Hour,
    ) -> serde_json::Value {
        loop {
            if let Some(at) = seen.iter().position(|(id, hour, kind, _)| *id == target_id && *hour == target_hour && *kind == dated::DATA) {
                return seen.remove(at).3.expect("panel data");
            }
            if let Some((playhead, epoch)) = expected_epoch {
                assert_eq!(playhead.epoch(), epoch, "playhead reset while waiting for panel {target_id} hour {target_hour}");
            }
            let (kind, bytes, session) = push.take_sent().expect("panel packet");
            push.credit(session, u64::MAX);
            if kind != PANEL {
                continue;
            }
            for r in dated::records(&bytes).unwrap() {
                let value = (r.kind == dated::DATA).then(|| rmp_serde::from_slice(r.body).expect("panel value"));
                seen.push((r.id, r.hour, r.kind, value));
            }
        }
    }

    fn anchored(playhead: &Playhead, anchor: Anchor) {
        playhead.frame_rate(anchor.frame_rate);
        playhead.pace(anchor.speed);
        if anchor.sought {
            playhead.seek(anchor.hour);
        }
        if !anchor.playing {
            playhead.pause();
        }
    }

    fn open_host(
        pages: &Arc<Recording>,
        pieces: Vec<playhead::Piece>,
        hook_fn: impl FnMut(&StepStats, Option<Message>) + Send + 'static,
    ) -> (Arc<Playhead>, Arc<PlayheadHost<Playhead>>, Arc<Push>) {
        let timeline = Timeline::new(&**pages, pieces).unwrap();
        let follow = Follow::new(0);
        let hook = follow.hook(Box::new(hook_fn));
        let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 32, ring: 1, speed: 1.0, rate: 60.0, ..Settings::default() };
        let playhead = Arc::new(Playhead::new(Arc::clone(pages) as Arc<dyn Pages>, timeline, settings, Some(hook)).unwrap());
        let held = crate::host::held_tables(&**pages, pages.kinds.len());
        let host = PlayheadHost::new(Arc::clone(&playhead), |p| p, anchored, follow, held).unwrap();
        let push = Arc::new(Push::default());
        host.show_panels(Arc::new(NoRows), Arc::clone(&push));
        (playhead, host, push)
    }

    #[test]
    fn a_stopped_reader_says_so_as_a_failure_and_recovers_and_never_says_not_recorded() {
        use replay_frame::frame::{DecodedFrame, Metrics};
        #[derive(Default)]
        struct Feed {
            frame: Option<Arc<DecodedFrame>>,
            wake: Option<crate::panel::Wake>,
            stopped: Option<(Hour, String)>,
        }
        struct FeedRows {
            playhead: Arc<Playhead>,
            feed: Arc<Mutex<Feed>>,
        }
        impl FeedRows {
            fn set(&self, change: impl FnOnce(&mut Feed)) {
                let wake = {
                    let mut feed = self.feed.lock().unwrap();
                    change(&mut feed);
                    feed.wake.clone()
                };
                if let Some(wake) = wake {
                    wake();
                }
            }
            fn frame(hour: Hour) -> Arc<DecodedFrame> {
                Arc::new(DecodedFrame { branch: 1, hour, tables: std::array::from_fn(|_| replay_frame::frame::not_selected()), metrics: Metrics::default() })
            }
            fn publish(&self, hour: Hour) {
                self.set(|feed| feed.frame = Some(Self::frame(hour)));
            }
        }
        struct FeedReader {
            playhead: Arc<Playhead>,
            feed: Arc<Mutex<Feed>>,
        }
        impl crate::panel::PanelReader for FeedReader {
            fn frame(&self) -> Option<Arc<DecodedFrame>> {
                self.feed.lock().unwrap().frame.clone()
            }
            fn through_seek(&self, frame: &Arc<DecodedFrame>) -> Option<(Hour, u64)> {
                Some((frame.hour, self.playhead.epoch()))
            }
            fn stopped(&self) -> Option<(Hour, String)> {
                self.feed.lock().unwrap().stopped.clone()
            }
        }
        impl PanelRows for FeedRows {
            fn reader(&self, _tables: &[TableId]) -> io::Result<Box<dyn crate::panel::PanelReader>> {
                Ok(Box::new(FeedReader { playhead: Arc::clone(&self.playhead), feed: Arc::clone(&self.feed) }))
            }
            fn on_publish(&self, wake: crate::panel::Wake) {
                self.feed.lock().unwrap().wake = Some(wake);
            }
        }
        struct Hourly;
        impl crate::panel::Panel for Hourly {
            fn tables(&self) -> &[TableId] { &[] }
            fn data(&mut self, frame: &DecodedFrame) -> crate::panel::PanelData {
                Ok(serde_json::json!({ "hour": frame.hour }))
            }
        }
        fn next_record(push: &Push, id: u32) -> (Hour, u8, Vec<u8>) {
            loop {
                let (kind, bytes, session) = push.take_sent().expect("panel packet");
                push.credit(session, u64::MAX);
                if kind != PANEL {
                    continue;
                }
                if let Some(r) = dated::records(&bytes).unwrap().into_iter().find(|r| r.id == id && [dated::DATA, dated::REFUSED, dated::STOPPED].contains(&r.kind)) {
                    return (r.hour, r.kind, r.body.to_vec());
                }
            }
        }

        let pages = record(quiet);
        let timeline = Timeline::new(&*pages, vec![playhead::Piece { run: 0, first: 0, last: 12 }]).unwrap();
        let follow = Follow::new(0);
        let hook = follow.hook(Box::new(|_, _| {}));
        let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 32, ring: 1, speed: 1.0, rate: 60.0, ..Settings::default() };
        let playhead = Arc::new(Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, timeline, settings, Some(hook)).unwrap());
        let held = crate::host::held_tables(&*pages, pages.kinds.len());
        let host = PlayheadHost::new(Arc::clone(&playhead), |p| p, anchored, follow, held).unwrap();
        let rows = Arc::new(FeedRows { playhead: Arc::clone(&playhead), feed: Arc::default() });
        let push = Arc::new(Push::default());
        host.show_panels(Arc::clone(&rows) as Arc<dyn PanelRows>, Arc::clone(&push));
        host.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        playhead.wait_newest(0).unwrap();

        host.prepare_panel(Box::new(Hourly)).unwrap()(42, 1).unwrap();
        rows.publish(0);
        assert_eq!(next_record(&push, 42).1, dated::DATA);

        rows.set(|feed| feed.stopped = Some((0, "the recording file was closed".into())));
        let (_, kind, body) = next_record(&push, 42);
        assert_eq!(kind, dated::STOPPED, "a stopped playhead is a failure, never a refusal by the recording");
        assert_eq!(std::str::from_utf8(&body).unwrap(), "the recording file was closed", "the reason as the playhead gave it");

        rows.set(|feed| (feed.stopped, feed.frame) = (None, Some(FeedRows::frame(1))));
        let (hour, kind, _) = next_record(&push, 42);
        assert_eq!((hour, kind), (1, dated::DATA), "the panel reads again once the playhead plays again");
    }

    #[test]
    fn paused_cells_dropping_across_a_gap_do_not_reset_the_playhead() {
        let pages = record(quiet);
        let (playhead, host, push) = open_host(
            &pages,
            vec![playhead::Piece { run: 0, first: 0, last: 3 }, playhead::Piece { run: 0, first: 5, last: 24 }],
            |_, _| {},
        );
        let mut seen = Vec::new();
        host.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        playhead.wait_newest(0).unwrap();

        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            if hour == 4 {
                ProductRead { value: Arc::new(Ok(serde_json::json!({ "gap": true }))), rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new(), from: Hour::MIN }
            } else {
                let (live, value) = live_cell(0, 100, i64::from(hour) * 10);
                ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
            }
        }))).unwrap()(10, 1).unwrap();

        let seeded = take_panel_data(&mut seen, &push, 10, 0);
        assert_eq!(seeded["live"]["groups"][0]["seedHour"], 0);
        assert!(seeded["live"]["groups"][0]["follow"].is_u64());
        playhead.wait_declarations().unwrap();
        let epoch_after_settle = playhead.epoch();
        playhead.pace(16.0);

        let returned = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_after_settle)), 10, 5);
        assert!(returned["live"]["groups"][0]["follow"].is_u64(), "returning cell group after gap receives a follow id");
        assert_eq!(playhead.epoch(), epoch_after_settle, "cells returning after a gap must not reset the paused playhead");
        playhead.wait_newest(8).unwrap();
        playhead.wait_declarations().unwrap();
        assert_eq!(playhead.epoch(), epoch_after_settle, "cells dropping at the gap and returning after it must cause 0 resets after first settle");
        assert_eq!(playhead.held().2.joins_open, 0);
    }

    #[test]
    fn paused_catch_up_returning_cells_across_gap_do_not_reset_playhead() {
        let pages = record(quiet);
        let (playhead, host, push) = open_host(
            &pages,
            vec![playhead::Piece { run: 0, first: 0, last: 3 }, playhead::Piece { run: 0, first: 5, last: 24 }],
            |_, _| {},
        );
        let mut seen = Vec::new();
        host.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        playhead.wait_newest(0).unwrap();

        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(0, 100, i64::from(hour) * 10);
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(10, 1).unwrap();
        let _ = take_panel_data(&mut seen, &push, 10, 0);
        playhead.wait_declarations().unwrap();
        let epoch_after_settle = playhead.epoch();

        let (seed_entered_tx, seed_entered_rx) = mpsc::channel();
        let (seed_release_tx, seed_release_rx) = mpsc::channel();
        let seed_release_rx = Mutex::new(seed_release_rx);
        let catch_up_hour_5 = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let catch_up_hour_5_read = Arc::clone(&catch_up_hour_5);
        host.prepare_panel(product(Arc::new(move |hour, values, _, step| {
            if hour == 0 && step.is_none() {
                let _ = seed_entered_tx.send(());
                let _ = seed_release_rx.lock().unwrap().recv();
                return ProductRead { value: Arc::new(Ok(serde_json::json!({ "init": true }))), rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new(), from: Hour::MIN };
            }
            if hour == 4 {
                return ProductRead { value: Arc::new(Ok(serde_json::json!({ "gap": true }))), rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new(), from: Hour::MIN };
            }
            if hour == 5 && step.is_none() {
                catch_up_hour_5_read.store(true, std::sync::atomic::Ordering::Release);
                if !values {
                    let (live, value) = live_cell(0, 100, 40);
                    return ProductRead { value, rows: 1, fresh: false, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN };
                }
            }
            let (live, value) = live_cell(0, 100, i64::from(hour) * 10);
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(11, 1).unwrap();

        seed_entered_rx.recv().unwrap();
        playhead.pace(16.0);
        playhead.wait_newest(8).unwrap();
        seed_release_tx.send(()).unwrap();

        let _ = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_after_settle)), 11, 0);
        let returned = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_after_settle)), 11, 5);
        assert!(returned["live"]["groups"][0]["follow"].is_u64(), "returning cell group in catch-up receives a follow id");
        assert_eq!(returned["live"]["groups"][0]["seedHour"], 5, "cells a shared reader returns from its cache are read again with their values at the catch-up hour");
        assert_eq!(returned["live"]["groups"][0]["values"], serde_json::json!([50]));
        loop {
            if seen.iter().any(|(id, hour, kind, _)| *id == 11 && *hour == 8 && (*kind == dated::DATA || *kind == dated::STANDS)) {
                break;
            }
            assert_eq!(playhead.epoch(), epoch_after_settle, "playhead reset while waiting for panel 11 catch-up hour 8");
            let (kind, bytes, session) = push.take_sent().expect("panel packet");
            push.credit(session, u64::MAX);
            if kind != PANEL {
                continue;
            }
            for r in dated::records(&bytes).unwrap() {
                let value = (r.kind == dated::DATA).then(|| rmp_serde::from_slice(r.body).expect("panel value"));
                seen.push((r.id, r.hour, r.kind, value));
            }
        }
        playhead.wait_declarations().unwrap();
        assert!(catch_up_hour_5.load(std::sync::atomic::Ordering::Acquire), "hour 5 ran on PanelRead::CatchUp (step = None)");
        assert_eq!(playhead.epoch(), epoch_after_settle, "catch-up registering a cell on an already-retained column must cause 0 paused resets");
    }

    #[test]
    fn replaced_panel_request_replaces_retained_lanes_and_unpanel_clears_them() {
        let pages = record(quiet);
        let (playhead, host, push) = open_host(
            &pages,
            vec![playhead::Piece { run: 0, first: 0, last: 12 }],
            |_, _| {},
        );
        let mut seen = Vec::new();
        host.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        playhead.wait_newest(0).unwrap();

        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(3, 105, i64::from(hour));
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(40, 1).unwrap();
        let _ = take_panel_data(&mut seen, &push, 40, 0);
        playhead.wait_declarations().unwrap();
        assert_eq!(
            host.follow().declared().unwrap(),
            [playhead::Declared { table: TABLE, columns: vec![3], keyed: false, passing: true, hourly: false }],
            "initial panel retains its live-cell column"
        );

        seen.clear();
        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(0, 100, i64::from(hour));
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(40, 1).unwrap();
        let _ = take_panel_data(&mut seen, &push, 40, 0);
        playhead.wait_declarations().unwrap();
        assert_eq!(
            host.follow().declared().unwrap(),
            [playhead::Declared { table: TABLE, columns: vec![0], keyed: false, passing: true, hourly: false }],
            "replacing the request on the same panel id replaces its retained lanes instead of merging into old ones"
        );

        host.unpanel(40);
        playhead.wait_declarations().unwrap();
        assert!(host.follow().declared().unwrap().is_empty(), "removing the panel clears its retained lanes");
    }

    #[test]
    fn shown_panel_retains_cell_lanes_replaces_on_reinstall_and_clears_on_unpanel() {
        use replay_frame::frame::{DecodedFrame, Metrics};
        struct FeedRows {
            playhead: Arc<Playhead>,
            state: Arc<Mutex<(Option<Arc<DecodedFrame>>, Option<crate::panel::Wake>)>>,
        }
        impl FeedRows {
            fn publish(&self, hour: Hour) {
                let frame = Arc::new(DecodedFrame {
                    branch: 1,
                    hour,
                    tables: std::array::from_fn(|_| replay_frame::frame::not_selected()),
                    metrics: Metrics::default(),
                });
                let wake = {
                    let mut guard = self.state.lock().unwrap();
                    guard.0 = Some(frame);
                    guard.1.clone()
                };
                if let Some(wake) = wake {
                    wake();
                }
            }
        }
        struct FeedReader {
            playhead: Arc<Playhead>,
            state: Arc<Mutex<(Option<Arc<DecodedFrame>>, Option<crate::panel::Wake>)>>,
        }
        impl crate::panel::PanelReader for FeedReader {
            fn frame(&self) -> Option<Arc<DecodedFrame>> {
                self.state.lock().unwrap().0.clone()
            }
            fn through_seek(&self, frame: &Arc<DecodedFrame>) -> Option<(Hour, u64)> {
                Some((frame.hour, self.playhead.epoch()))
            }
        }
        impl PanelRows for FeedRows {
            fn reader(&self, _tables: &[TableId]) -> io::Result<Box<dyn crate::panel::PanelReader>> {
                Ok(Box::new(FeedReader { playhead: Arc::clone(&self.playhead), state: Arc::clone(&self.state) }))
            }
            fn on_publish(&self, wake: crate::panel::Wake) {
                self.state.lock().unwrap().1 = Some(wake);
            }
        }
        struct ShownCellPanel {
            column: usize,
            key: u128,
            drop_at: Option<Hour>,
            live: Option<crate::live::Live>,
        }
        impl crate::panel::Panel for ShownCellPanel {
            fn tables(&self) -> &[TableId] { &[] }
            fn data(&mut self, frame: &DecodedFrame) -> crate::panel::PanelData {
                if self.drop_at == Some(frame.hour) {
                    self.live = None;
                    return Ok(serde_json::json!({ "gap": true }));
                }
                let cells = Cells::default();
                cells.value("r", "v", Source { table: TableId::ALL[TABLE], column: self.column, key: self.key }, 1.0);
                self.live = Some(cells.finish());
                Ok(serde_json::json!({ "ok": true }))
            }
            fn live(&self) -> Option<&crate::live::Live> {
                self.live.as_ref()
            }
        }

        let pages = record(quiet);
        let timeline = Timeline::new(&*pages, vec![playhead::Piece { run: 0, first: 0, last: 12 }]).unwrap();
        let follow = Follow::new(0);
        let hook = follow.hook(Box::new(|_, _| {}));
        let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 32, ring: 1, speed: 1.0, rate: 60.0, ..Settings::default() };
        let playhead = Arc::new(Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, timeline, settings, Some(hook)).unwrap());
        let held = crate::host::held_tables(&*pages, pages.kinds.len());
        let host = PlayheadHost::new(Arc::clone(&playhead), |p| p, anchored, follow, held).unwrap();
        let rows = Arc::new(FeedRows { playhead: Arc::clone(&playhead), state: Arc::new(Mutex::new((None, None))) });
        let push = Arc::new(Push::default());
        host.show_panels(Arc::clone(&rows) as Arc<dyn PanelRows>, Arc::clone(&push));

        let mut seen = Vec::new();
        host.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        playhead.wait_newest(0).unwrap();

        host.prepare_panel(Box::new(ShownCellPanel { column: 3, key: 105, drop_at: Some(1), live: None })).unwrap()(41, 1).unwrap();
        rows.publish(0);
        let _ = take_panel_data(&mut seen, &push, 41, 0);
        playhead.wait_declarations().unwrap();
        assert_eq!(
            host.follow().declared().unwrap(),
            [playhead::Declared { table: TABLE, columns: vec![3], keyed: false, passing: true, hourly: false }],
            "shown panel retains its live-cell column"
        );

        rows.publish(1);
        let _ = take_panel_data(&mut seen, &push, 41, 1);
        playhead.wait_declarations().unwrap();
        assert_eq!(
            host.follow().declared().unwrap(),
            [playhead::Declared { table: TABLE, columns: vec![3], keyed: false, passing: true, hourly: false }],
            "shown panel keeps its retained cell column when a frame has no live cells"
        );

        seen.clear();
        host.prepare_panel(Box::new(ShownCellPanel { column: 0, key: 100, drop_at: None, live: None })).unwrap()(41, 1).unwrap();
        rows.publish(2);
        let _ = take_panel_data(&mut seen, &push, 41, 2);
        playhead.wait_declarations().unwrap();
        assert_eq!(
            host.follow().declared().unwrap(),
            [playhead::Declared { table: TABLE, columns: vec![0], keyed: false, passing: true, hourly: false }],
            "re-installing a shown panel on the same id replaces its retained cell lanes"
        );

        host.unpanel(41);
        playhead.wait_declarations().unwrap();
        assert!(host.follow().declared().unwrap().is_empty(), "unpanel clears retained lanes for a shown panel");
    }

    #[test]
    fn paused_cross_piece_table_join_and_shrink_seek_once() {
        let pages = record(quiet);
        let (playhead, host, push) = open_host(
            &pages,
            vec![playhead::Piece { run: 0, first: 0, last: 3 }, playhead::Piece { run: 0, first: 5, last: 24 }],
            |_, _| {},
        );
        let mut seen = Vec::new();
        host.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        playhead.wait_newest(0).unwrap();
        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(0, 100, i64::from(hour) * 10);
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(10, 1).unwrap();
        let _ = take_panel_data(&mut seen, &push, 10, 0);
        playhead.wait_declarations().unwrap();
        playhead.pace(16.0);
        playhead.wait_newest(8).unwrap();
        playhead.wait_declarations().unwrap();

        let epoch_before_cross_piece = playhead.epoch();
        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            if hour < 5 {
                ProductRead { value: Arc::new(Ok(serde_json::json!({ "empty": true }))), rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new(), from: Hour::MIN }
            } else {
                let (live, value) = live_cell(3, 105, i64::from(hour));
                ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
            }
        }))).unwrap()(11, 1).unwrap();
        let joined = take_panel_data(&mut seen, &push, 11, 5);
        assert!(joined["live"]["groups"][0]["follow"].is_u64());
        playhead.wait_newest(8).unwrap();
        playhead.wait_declarations().unwrap();
        assert_eq!(playhead.epoch(), epoch_before_cross_piece + 1, "a new cross-piece column join takes exactly one clean seek and terminates");
        assert_eq!(playhead.held().2.joins_open, 0);
        drop(host);

        let (playhead2, host2, push2) = open_host(
            &pages,
            vec![playhead::Piece { run: 0, first: 0, last: 3 }, playhead::Piece { run: 0, first: 5, last: 12 }],
            |_, _| {},
        );
        let mut seen2 = Vec::new();
        host2.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        playhead2.wait_newest(0).unwrap();
        host2.prepare_panel(product(Arc::new(|hour, _, _, _| {
            if hour < 5 {
                ProductRead { value: Arc::new(Ok(serde_json::json!({ "before": true }))), rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new(), from: Hour::MIN }
            } else {
                let (live, value) = live_cell(3, 105, i64::from(hour));
                ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
            }
        }))).unwrap()(20, 1).unwrap();
        let _ = take_panel_data(&mut seen2, &push2, 20, 0);
        let epoch_before_boundary = playhead2.epoch();
        assert!(playhead2.newest().is_some_and(|n| n < 5), "panel is installed before read-ahead crosses the gap");
        playhead2.pace(24.0);
        let boundary_joined = take_panel_data(&mut seen2, &push2, 20, 5);
        assert!(boundary_joined["live"]["groups"][0]["follow"].is_u64());
        playhead2.wait_newest(8).unwrap();
        playhead2.wait_declarations().unwrap();
        assert_eq!(playhead2.epoch(), epoch_before_boundary + 1, "a column first declared right at the piece boundary takes exactly one clean seek and settles");
        assert_eq!(playhead2.held().2.joins_open, 0);
        drop(host2);

        let gate3 = SinkGate::new();
        let gate3_hook = Arc::clone(&gate3);
        let (playhead3, host3, push3) = open_host(
            &pages,
            vec![playhead::Piece { run: 0, first: 0, last: 3 }, playhead::Piece { run: 0, first: 5, last: 12 }],
            move |stats, _| gate3_hook.hit(stats.last),
        );
        let mut seen3 = Vec::new();
        host3.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        playhead3.wait_newest(0).unwrap();
        host3.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(0, 100, i64::from(hour));
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(30, 1).unwrap();
        host3.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(3, 105, i64::from(hour));
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(31, 1).unwrap();
        let _ = take_panel_data(&mut seen3, &push3, 30, 0);
        let _ = take_panel_data(&mut seen3, &push3, 31, 0);
        playhead3.wait_declarations().unwrap();
        gate3.arm(5);
        playhead3.pace(10.0);
        gate3.wait_entered();
        let epoch_at_piece1 = playhead3.epoch();
        host3.unpanel(31);
        gate3.release();
        playhead3.wait_declarations().unwrap();
        playhead3.wait_newest(5).unwrap();
        assert_eq!(playhead3.epoch(), epoch_at_piece1 + 1, "shrinking a retained table as piece 1 fills the ring wakes the driver and settles cleanly");
        assert_eq!(playhead3.held().2.joins_open, 0);
    }

    #[test]
    fn external_paused_follow_still_seeds_at_paused_hour_without_redundant_resets() {
        let pages = record(quiet);
        let (follow_tx, follow_rx) = mpsc::channel();
        let (playhead, host, push) = open_host(
            &pages,
            vec![playhead::Piece { run: 0, first: 0, last: 3 }, playhead::Piece { run: 0, first: 5, last: 24 }],
            move |_, message| {
                if let Some(message) = message {
                    let _ = follow_tx.send(message.bytes);
                }
            },
        );
        let mut seen = Vec::new();
        host.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        playhead.wait_newest(0).unwrap();
        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(0, 100, i64::from(hour) * 10);
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(10, 1).unwrap();
        let _ = take_panel_data(&mut seen, &push, 10, 0);
        playhead.wait_declarations().unwrap();
        playhead.pace(16.0);
        playhead.wait_newest(8).unwrap();
        playhead.wait_declarations().unwrap();

        let epoch_before_external = playhead.epoch();
        host.follow().follow(99, Query::Cells { table: TABLE, columns: vec![0], rows: Rows::Keys(vec![100]) }).unwrap();
        let seeded_answer = loop {
            let bytes = follow_rx.recv().expect("follow message");
            let decoded = decode(&bytes).expect("valid follow message");
            if let Some((_, _, body)) = decoded.answers.into_iter().find(|&(id, hour, _)| id == 99 && hour == 0) {
                break Answer::parse(body).expect("valid answer");
            }
        };
        assert!(playhead.epoch() > epoch_before_external, "a real new subscriber while paused still triggers its paused seed");
        assert_eq!(seeded_answer.values, vec![0]);
        drop(host);

        let gate4 = SinkGate::new();
        let gate4_hook = Arc::clone(&gate4);
        let (follow_tx4, follow_rx4) = mpsc::channel();
        let (playhead4, host4, _push4) = open_host(
            &pages,
            vec![playhead::Piece { run: 0, first: 0, last: 12 }],
            move |stats, message| {
                if let Some(message) = message {
                    let _ = follow_tx4.send(message.bytes);
                }
                gate4_hook.hit(stats.last);
            },
        );
        host4.follow().follow(1, Query::Cells { table: TABLE, columns: vec![0], rows: Rows::Keys(vec![100]) }).unwrap();
        gate4.arm(1);
        host4.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        loop {
            let bytes = follow_rx4.recv().expect("initial follow message");
            if decode(&bytes).unwrap().answers.iter().any(|&(id, hour, _)| id == 1 && hour == 0) {
                break;
            }
        }
        playhead4.pace(4.0);
        gate4.wait_entered();
        playhead4.seek(0);
        let held_epoch = playhead4.epoch();
        assert_eq!(playhead4.holding(), Some(held_epoch));
        host4.follow().follow(2, Query::Cells { table: TABLE, columns: vec![0], rows: Rows::Keys(vec![100]) }).unwrap();
        assert_eq!(playhead4.epoch(), held_epoch, "registration during a held seek does not seek before the held reset runs");
        gate4.release();
        let held_answer = loop {
            let bytes = follow_rx4.recv().expect("held reset follow message");
            let decoded = decode(&bytes).expect("valid follow message");
            if let Some((_, _, body)) = decoded.answers.into_iter().find(|&(id, hour, _)| id == 2 && hour == 0) {
                break Answer::parse(body).expect("valid answer");
            }
        };
        assert_eq!(held_answer.values, vec![0]);
        playhead4.pace(16.0);
        playhead4.wait_newest(3).unwrap();
        playhead4.wait_declarations().unwrap();
        assert_eq!(playhead4.epoch(), held_epoch, "once the held reset satisfies the subscriber, advancing read-ahead causes 0 redundant resets");

        gate4.arm(0);
        playhead4.pace(1.0);
        playhead4.seek(0);
        let reset_sink_epoch = playhead4.epoch();
        gate4.wait_entered();
        host4.follow().follow(3, Query::Cells { table: TABLE, columns: vec![0], rows: Rows::Keys(vec![100]) }).unwrap();
        gate4.release();
        let rehook_answer = loop {
            let bytes = follow_rx4.recv().expect("rehook follow message");
            let decoded = decode(&bytes).expect("valid follow message");
            if let Some((_, _, body)) = decoded.answers.into_iter().find(|&(id, hour, _)| id == 3 && hour == 0) {
                break Answer::parse(body).expect("valid answer");
            }
        };
        assert_eq!(rehook_answer.values, vec![0]);
        playhead4.pace(16.0);
        playhead4.wait_newest(3).unwrap();
        playhead4.wait_declarations().unwrap();
        assert_eq!(playhead4.epoch(), reset_sink_epoch, "a subscriber registered during the reset sink and answered on the next hook causes 0 redundant resets");
    }

    #[test]
    fn paused_exact_seeded_panel_causes_no_reset_and_delivers_catchup_values() {
        let pages = record(quiet);
        let (follow_tx, follow_rx) = mpsc::channel();
        let (playhead, host, push) = open_host(
            &pages,
            vec![playhead::Piece { run: 0, first: 0, last: 3 }, playhead::Piece { run: 0, first: 5, last: 24 }],
            move |_, message| {
                if let Some(message) = message {
                    let _ = follow_tx.send(message.bytes);
                }
            },
        );
        let mut seen = Vec::new();
        host.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        playhead.wait_newest(0).unwrap();

        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(0, 100, i64::from(hour) * 10);
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(10, 1).unwrap();
        let _ = take_panel_data(&mut seen, &push, 10, 0);
        playhead.wait_declarations().unwrap();
        playhead.pace(16.0);
        playhead.wait_newest(8).unwrap();
        playhead.wait_declarations().unwrap();

        let epoch_before_same_table_seed = playhead.epoch();
        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(0, 101, i64::from(hour) * 10 + 1);
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(12, 1).unwrap();

        let seeded_extra = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_before_same_table_seed)), 12, 0);
        assert_eq!(seeded_extra["live"]["groups"][0]["seedHour"], 0);
        assert_eq!(seeded_extra["live"]["groups"][0]["values"], serde_json::json!([1]));
        assert!(seeded_extra["live"]["groups"][0]["follow"].is_u64());
        let catchup_extra = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_before_same_table_seed)), 12, 3);
        assert_eq!(catchup_extra["live"]["groups"][0]["seedHour"], 3, "catch-up delivers dated cell values before follow coverage starts");
        assert_eq!(catchup_extra["live"]["groups"][0]["values"], serde_json::json!([31]));
        playhead.wait_newest(8).unwrap();
        playhead.wait_declarations().unwrap();
        assert_eq!(playhead.epoch(), epoch_before_same_table_seed, "an exact-seeded panel on an already-declared column causes 0 resets");

        let cached_wide: Arc<Mutex<Option<(Arc<crate::live::Live>, Arc<crate::panel::PanelData>)>>> = Arc::new(Mutex::new(None));
        let cached_wide_ref = Arc::clone(&cached_wide);
        host.prepare_panel(product(Arc::new(move |hour, want_values, _, _| {
            let mut slot = cached_wide_ref.lock().unwrap();
            let (live, value) = match (&*slot, want_values) {
                (Some(held), false) => held.clone(),
                _ => {
                    let val = if (3..=4).contains(&hour) { 9_007_199_254_740_990 + i64::from(hour) } else { i64::from(hour) * 10 + 3 };
                    let built = live_cell(0, 101, val);
                    *slot = Some(built.clone());
                    built
                }
            };
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(13, 1).unwrap();
        let wide_h0 = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_before_same_table_seed)), 13, 0);
        assert_eq!(wide_h0["live"]["groups"][0]["seedHour"], 0);
        assert!(wide_h0["live"]["groups"][0]["follow"].is_u64());
        let wide_h3 = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_before_same_table_seed)), 13, 3);
        assert_eq!(wide_h3["live"]["groups"][0]["rawValues"], serde_json::json!(["9007199254740993"]), "wide catch-up hour delivers lossless rawValues");
        assert!(wide_h3["live"]["groups"][0].get("follow").is_none(), "wide catch-up hour does not strip values into an unseeded follow");
        let wide_h4 = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_before_same_table_seed)), 13, 4);
        assert_eq!(wide_h4["live"]["groups"][0]["rawValues"], serde_json::json!(["9007199254740994"]), "persistent wide catch-up refreshes values instead of reusing cached Arc");
        let wide_h5 = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_before_same_table_seed)), 13, 5);
        assert_eq!(wide_h5["live"]["groups"][0]["seedHour"], 5);
        assert_eq!(wide_h5["live"]["groups"][0]["values"], serde_json::json!([53]));
        assert!(wide_h5["live"]["groups"][0]["follow"].is_u64());
        assert_eq!(playhead.epoch(), epoch_before_same_table_seed, "safe->wide->safe catch-up transition causes 0 paused resets");

        let cached_end_wide: Arc<Mutex<Option<(Arc<crate::live::Live>, Arc<crate::panel::PanelData>)>>> = Arc::new(Mutex::new(None));
        let cached_end_wide_ref = Arc::clone(&cached_end_wide);
        host.prepare_panel(product(Arc::new(move |hour, want_values, _, _| {
            let mut slot = cached_end_wide_ref.lock().unwrap();
            let (live, value) = match (&*slot, want_values) {
                (Some(held), false) => held.clone(),
                _ => {
                    let val = if hour >= 8 { 9_007_199_254_740_990 + i64::from(hour) } else { i64::from(hour) * 10 + 7 };
                    let built = live_cell(0, 105, val);
                    *slot = Some(built.clone());
                    built
                }
            };
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(15, 1).unwrap();
        let end_wide_h0 = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_before_same_table_seed)), 15, 0);
        let end_wide_follow = end_wide_h0["live"]["groups"][0]["follow"].as_u64().expect("initial follow id");
        let end_wide_h8 = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_before_same_table_seed)), 15, 8);
        assert_eq!(end_wide_h8["live"]["groups"][0]["rawValues"], serde_json::json!(["9007199254740998"]), "final catch-up hour delivers lossless wide inline value");
        assert!(end_wide_h8["live"]["groups"][0].get("follow").is_none());
        playhead.pace(32.0);
        playhead.wait_newest(10).unwrap();
        let resumed_step = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_before_same_table_seed)), 15, 9);
        let restored_follow = resumed_step["live"]["groups"][0]["follow"].as_u64().expect("restored follow id");
        assert_ne!(restored_follow, end_wide_follow, "restoring follow after an inline wide catch-up interval allocates a fresh follow id");
        assert!(resumed_step["live"]["groups"][0].get("values").is_none());
        assert!(resumed_step["live"]["groups"][0].get("rawValues").is_none());
        let mut restored_answer = None;
        for bytes in follow_rx.try_iter() {
            let decoded = decode(&bytes).expect("valid follow message");
            if let Some((_, _, body)) = decoded.answers.into_iter().find(|&(id, hour, _)| u64::from(id) == restored_follow && hour == 9) {
                assert!(
                    !decoded.limited.iter().any(|&(id, through)| u64::from(id) == restored_follow && through < 9),
                    "restored follow id must not be limited below hour 9: {:?}",
                    decoded.limited
                );
                assert!(decoded.hour >= 9);
                restored_answer = Some(Answer::parse(body).expect("valid follow answer"));
                break;
            }
        }
        let restored_answer = restored_answer.expect("restored follow id received a complete answer at hour 9");
        assert!(!restored_answer.changed, "restored follow id receives a complete answer (not a delta) at hour 9");
        assert_eq!(restored_answer.values, vec![0], "fresh follow id receives a complete answer at hour 9 even when the underlying cell value is unchanged");
        assert_eq!(playhead.epoch(), epoch_before_same_table_seed);

        let catchup_seeds = Arc::new(Mutex::new(Vec::new()));
        let catchup_seeds_ref = Arc::clone(&catchup_seeds);
        host.prepare_panel(product(Arc::new(move |hour, seed, _, step| {
            if step.is_none() && (1..=8).contains(&hour) {
                catchup_seeds_ref.lock().unwrap().push((hour, seed));
            }
            ProductRead { value: Arc::new(Ok(serde_json::json!({ "interval": hour }))), rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(14, 1).unwrap();
        let _interval_h8 = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_before_same_table_seed)), 14, 8);
        let seen_seeds = catchup_seeds.lock().unwrap().clone();
        assert!(!seen_seeds.is_empty() && seen_seeds.iter().all(|&(_, seed)| !seed), "quantum-1 catch-up preserves interval reuse (seed=false) for panels without live cells: {seen_seeds:?}");
        assert_eq!(playhead.epoch(), epoch_before_same_table_seed);

        let other_reader = playhead
            .declare(vec![playhead::Declared { table: TABLE, columns: vec![0, 3], keyed: false, passing: false, hourly: false }], None)
            .unwrap();
        playhead.wait_newest(8).unwrap();
        playhead.wait_declarations().unwrap();
        let epoch_before_covered = playhead.epoch();
        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(3, 105, i64::from(hour));
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(16, 1).unwrap();
        let covered_seed = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_before_covered)), 16, 0);
        assert_eq!(covered_seed["live"]["groups"][0]["seedHour"], 0);
        playhead.wait_declarations().unwrap();
        assert_eq!(playhead.epoch(), epoch_before_covered, "a declaration already covered by another reader causes 0 seeks across a piece boundary");
        assert_eq!(playhead.held().2.joins_open, 0);
        host.unpanel(16);
        playhead.redeclare(other_reader, Vec::new()).unwrap();
        playhead.wait_declarations().unwrap();
    }

    #[test]
    fn paused_cold_open_siblings_suffer_zero_relapses_when_late_unseeded_consumer_seeks() {
        let pages = record(quiet);
        let (follow_tx, follow_rx) = mpsc::channel();
        let (playhead, host, push) = open_host(
            &pages,
            vec![playhead::Piece { run: 0, first: 0, last: 3 }, playhead::Piece { run: 0, first: 5, last: 24 }],
            move |_, message| {
                if let Some(message) = message {
                    let _ = follow_tx.send(message.bytes);
                }
            },
        );
        let mut seen = Vec::new();
        host.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        playhead.wait_newest(0).unwrap();

        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(0, 100, i64::from(hour) * 10);
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(100, 1).unwrap();
        let _ = take_panel_data(&mut seen, &push, 100, 0);
        playhead.wait_declarations().unwrap();
        playhead.pace(16.0);
        playhead.wait_newest(8).unwrap();
        playhead.wait_declarations().unwrap();

        let epoch_after_warm = playhead.epoch();
        for id in [101u32, 102] {
            let offset = i64::from(id);
            host.prepare_panel(product(Arc::new(move |hour, _, _, _| {
                let (live, value) = live_cell(0, 100, i64::from(hour) * 10 + offset);
                ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
            }))).unwrap()(id, 1).unwrap();
            let data = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_after_warm)), id, 0);
            assert_eq!(data["live"]["groups"][0]["seedHour"], 0);
        }
        assert_eq!(playhead.epoch(), epoch_after_warm, "multiple seeded panels cold-opening while paused cause 0 seeks");

        let (inflight_entered_tx, inflight_entered_rx) = mpsc::channel();
        let (inflight_release_tx, inflight_release_rx) = mpsc::channel();
        let inflight_release_rx = Mutex::new(inflight_release_rx);
        let inflight_seed_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let inflight_seed_calls_ref = Arc::clone(&inflight_seed_calls);
        host.prepare_panel(product(Arc::new(move |hour, _, _, step| {
            if hour == 0 && step.is_none() {
                if inflight_seed_calls_ref.fetch_add(1, std::sync::atomic::Ordering::AcqRel) == 0 {
                    let _ = inflight_entered_tx.send(());
                    let _ = inflight_release_rx.lock().unwrap().recv();
                }
            }
            let (live, value) = live_cell(0, 100, i64::from(hour) * 10 + 103);
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(103, 1).unwrap();
        inflight_entered_rx.recv().unwrap();

        seen.clear();
        while let Some((kind, bytes, _, _)) = push.try_take() {
            push.credit(push.session(), u64::MAX);
            if kind == PANEL {
                for r in dated::records(&bytes).unwrap() {
                    let value = (r.kind == dated::DATA).then(|| rmp_serde::from_slice(r.body).expect("panel value"));
                    seen.push((r.id, r.hour, r.kind, value));
                }
            }
        }
        let mut waiting_after_settle: Vec<(u32, Hour)> = Vec::new();
        let mut waiting_beyond_delivery: Vec<(u32, Hour)> = Vec::new();
        let mut drain_checking = |seen: &mut Vec<(u32, Hour, u8, Option<serde_json::Value>)>, target_id: u32, target_hour: Hour| -> serde_json::Value {
            loop {
                if let Some(at) = seen.iter().position(|(id, hour, kind, _)| *id == target_id && *hour == target_hour && *kind == dated::DATA) {
                    return seen.remove(at).3.expect("panel data");
                }
                let (kind, bytes, session) = push.take_sent().expect("panel packet");
                push.credit(session, u64::MAX);
                if kind != PANEL {
                    continue;
                }
                for r in dated::records(&bytes).unwrap() {
                    if r.kind == dated::WAITING && (100..=103).contains(&r.id) {
                        if r.hour <= 0 {
                            waiting_after_settle.push((r.id, r.hour));
                        } else {
                            waiting_beyond_delivery.push((r.id, r.hour));
                        }
                    }
                    let value = (r.kind == dated::DATA).then(|| rmp_serde::from_slice(r.body).expect("panel value"));
                    seen.push((r.id, r.hour, r.kind, value));
                }
            }
        };

        seen.clear();
        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(0, 100, 9_007_199_254_740_995 + i64::from(hour));
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(104, 1).unwrap();
        let unseeded = drain_checking(&mut seen, 104, 0);
        let unseeded_follow = unseeded["live"]["groups"][0]["follow"].as_u64().expect("unseeded follow id") as u32;
        assert!(unseeded["live"]["groups"][0].get("seedHour").is_none(), "wide integer is unseeded");
        loop {
            let bytes = follow_rx.recv().expect("follow message for unseeded wide integer");
            let decoded = decode(&bytes).expect("valid follow message");
            if decoded.answers.into_iter().any(|(id, hour, _)| id == unseeded_follow && hour == 0) {
                break;
            }
        }
        assert_eq!(playhead.epoch(), epoch_after_warm + 1, "late unseeded wide-integer panel triggers exactly 1 paused seek");

        inflight_release_tx.send(()).unwrap();
        let inflight_data = drain_checking(&mut seen, 103, 0);
        assert_eq!(inflight_data["live"]["groups"][0]["seedHour"], 0);
        playhead.wait_newest(8).unwrap();
        playhead.wait_declarations().unwrap();
        while let Some((kind, bytes, _, _)) = push.try_take() {
            push.credit(push.session(), u64::MAX);
            if kind == PANEL {
                for r in dated::records(&bytes).unwrap() {
                    if r.kind == dated::WAITING && (100..=103).contains(&r.id) {
                        if r.hour <= 0 {
                            waiting_after_settle.push((r.id, r.hour));
                        } else {
                            waiting_beyond_delivery.push((r.id, r.hour));
                        }
                    }
                }
            }
        }
        assert_eq!(inflight_seed_calls.load(std::sync::atomic::Ordering::Acquire), 1, "in-flight sibling Seed(0) completed without being cancelled and re-run");
        assert!(waiting_after_settle.is_empty(), "settled and in-flight seeded siblings suffered zero paused-hour WAITING relapses during late unseeded seek: {waiting_after_settle:?}");
        assert!(waiting_beyond_delivery.iter().any(|&(id, hour)| id == 100 && hour > 8), "replacement seed emits WAITING beyond confirmed delivery so COMPLETED cannot extend stale hours: {waiting_beyond_delivery:?}");
    }

    #[test]
    fn shown_exact_seeded_panel_causes_no_paused_reset() {
        use replay_frame::frame::{DecodedFrame, Metrics, TableView};
        struct FeedRows {
            playhead: Arc<Playhead>,
            state: Arc<Mutex<(Option<(Arc<DecodedFrame>, Hour)>, Option<crate::panel::Wake>)>>,
        }
        impl FeedRows {
            fn publish(&self, hour: Hour, through: Hour) {
                let mut tables = std::array::from_fn(|_| replay_frame::frame::not_selected());
                tables[TABLE] = TableView::Complete(replay_data::rows::TableRows::Countries(Vec::new())).into();
                let frame = Arc::new(DecodedFrame { branch: 1, hour, tables, metrics: Metrics::default() });
                let wake = {
                    let mut guard = self.state.lock().unwrap();
                    guard.0 = Some((frame, through));
                    guard.1.clone()
                };
                if let Some(wake) = wake {
                    wake();
                }
            }
        }
        struct FeedReader {
            playhead: Arc<Playhead>,
            state: Arc<Mutex<(Option<(Arc<DecodedFrame>, Hour)>, Option<crate::panel::Wake>)>>,
        }
        impl crate::panel::PanelReader for FeedReader {
            fn frame(&self) -> Option<Arc<DecodedFrame>> { self.state.lock().unwrap().0.as_ref().map(|(f, _)| Arc::clone(f)) }
            fn through_seek(&self, frame: &Arc<DecodedFrame>) -> Option<(Hour, u64)> {
                let through = self.state.lock().unwrap().0.as_ref().map_or(frame.hour, |(_, through)| *through);
                Some((through, self.playhead.epoch()))
            }
        }
        impl PanelRows for FeedRows {
            fn reader(&self, _tables: &[TableId]) -> io::Result<Box<dyn crate::panel::PanelReader>> {
                Ok(Box::new(FeedReader { playhead: Arc::clone(&self.playhead), state: Arc::clone(&self.state) }))
            }
            fn on_publish(&self, wake: crate::panel::Wake) { self.state.lock().unwrap().1 = Some(wake); }
        }
        struct ShownSeedPanel(Option<crate::live::Live>);
        impl crate::panel::Panel for ShownSeedPanel {
            fn tables(&self) -> &[TableId] { &[] }
            fn data(&mut self, _frame: &DecodedFrame) -> crate::panel::PanelData {
                let cells = Cells::default();
                cells.value("r", "v", Source { table: TableId::ALL[TABLE], column: 0, key: 100 }, 1.0);
                self.0 = Some(cells.finish());
                Ok(serde_json::json!({ "ok": true }))
            }
            fn live(&self) -> Option<&crate::live::Live> { self.0.as_ref() }
        }

        let pages = record(quiet);
        let timeline = Timeline::new(&*pages, vec![playhead::Piece { run: 0, first: 0, last: 3 }, playhead::Piece { run: 0, first: 5, last: 24 }]).unwrap();
        let follow = Follow::new(0);
        let hook = follow.hook(Box::new(|_, _| {}));
        let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 32, ring: 1, speed: 1.0, rate: 60.0, ..Settings::default() };
        let playhead = Arc::new(Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, timeline, settings, Some(hook)).unwrap());
        let held = crate::host::held_tables(&*pages, pages.kinds.len());
        let host = PlayheadHost::new(Arc::clone(&playhead), |p| p, anchored, follow, held).unwrap();
        let rows = Arc::new(FeedRows { playhead: Arc::clone(&playhead), state: Arc::new(Mutex::new((None, None))) });
        let push = Arc::new(Push::default());
        host.show_panels(Arc::clone(&rows) as Arc<dyn PanelRows>, Arc::clone(&push));

        let mut seen = Vec::new();
        host.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        playhead.wait_newest(0).unwrap();

        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(0, 100, i64::from(hour) * 10);
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(10, 1).unwrap();
        let _ = take_panel_data(&mut seen, &push, 10, 0);
        playhead.wait_declarations().unwrap();
        playhead.pace(16.0);
        playhead.wait_newest(8).unwrap();
        playhead.wait_declarations().unwrap();

        let epoch_before_shown = playhead.epoch();
        host.prepare_panel(Box::new(ShownSeedPanel(None))).unwrap()(42, 1).unwrap();
        rows.publish(0, 0);
        let data = take_panel_data_at_epoch(&mut seen, &push, Some((&playhead, epoch_before_shown)), 42, 0);
        assert_eq!(data["live"]["groups"][0]["seedHour"], 0);
        assert!(data["live"]["groups"][0]["follow"].is_u64());
        playhead.wait_declarations().unwrap();
        assert_eq!(playhead.epoch(), epoch_before_shown, "an exact-seeded Shown panel on an already-declared column causes 0 paused resets");
    }

    #[test]
    fn shown_inflight_read_survives_external_paused_reset_with_one_evaluation() {
        use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TableView};
        struct FeedRows {
            playhead: Arc<Playhead>,
            shared: SharedView,
            state: Arc<Mutex<(Option<(Arc<DecodedFrame>, Hour, u64)>, Option<crate::panel::Wake>)>>,
        }
        impl FeedRows {
            fn publish(&self, hour: Hour, through: Hour, sampled_at: Option<Hour>, reuse_ptr: bool) {
                let mut tables = std::array::from_fn(|_| replay_frame::frame::not_selected());
                let view: SharedView = if reuse_ptr {
                    self.shared.clone()
                } else {
                    TableView::Complete(replay_data::rows::TableRows::Countries(Vec::new())).into()
                };
                tables[TABLE] = view.with_sampled_at(sampled_at);
                let frame = Arc::new(DecodedFrame { branch: 1, hour, tables, metrics: Metrics::default() });
                let epoch = self.playhead.epoch();
                let wake = {
                    let mut guard = self.state.lock().unwrap();
                    guard.0 = Some((frame, through, epoch));
                    guard.1.clone()
                };
                if let Some(wake) = wake {
                    wake();
                }
            }
        }
        struct FeedReader {
            state: Arc<Mutex<(Option<(Arc<DecodedFrame>, Hour, u64)>, Option<crate::panel::Wake>)>>,
        }
        impl crate::panel::PanelReader for FeedReader {
            fn frame(&self) -> Option<Arc<DecodedFrame>> { self.state.lock().unwrap().0.as_ref().map(|(f, _, _)| Arc::clone(f)) }
            fn through_seek(&self, frame: &Arc<DecodedFrame>) -> Option<(Hour, u64)> {
                let guard = self.state.lock().unwrap();
                let (held, through, epoch) = guard.0.as_ref()?;
                Arc::ptr_eq(held, frame).then_some((*through, *epoch))
            }
        }
        impl PanelRows for FeedRows {
            fn reader(&self, _tables: &[TableId]) -> io::Result<Box<dyn crate::panel::PanelReader>> {
                Ok(Box::new(FeedReader { state: Arc::clone(&self.state) }))
            }
            fn on_publish(&self, wake: crate::panel::Wake) { self.state.lock().unwrap().1 = Some(wake); }
        }
        struct GatedShownPanel {
            entered: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
            evals: Arc<std::sync::atomic::AtomicUsize>,
            live: Option<crate::live::Live>,
        }
        impl crate::panel::Panel for GatedShownPanel {
            fn tables(&self) -> &[TableId] { &[] }
            fn data(&mut self, frame: &DecodedFrame) -> crate::panel::PanelData {
                if self.evals.fetch_add(1, std::sync::atomic::Ordering::AcqRel) == 0 {
                    let _ = self.entered.send(());
                    let _ = self.release.lock().unwrap().recv();
                }
                let cells = Cells::default();
                cells.value("r", "v", Source { table: TableId::ALL[TABLE], column: 0, key: 100 }, 1.0);
                self.live = Some(cells.finish());
                Ok(serde_json::json!({ "ok": true, "sampledAt": frame.tables[TABLE].sampled_at() }))
            }
            fn live(&self) -> Option<&crate::live::Live> { self.live.as_ref() }
        }

        let pages = record(quiet);
        let (follow_tx, follow_rx) = mpsc::channel();
        let timeline = Timeline::new(&*pages, vec![playhead::Piece { run: 0, first: 0, last: 3 }, playhead::Piece { run: 0, first: 5, last: 24 }]).unwrap();
        let follow = Follow::new(0);
        let hook = follow.hook(Box::new(move |_, message| {
            if let Some(message) = message { let _ = follow_tx.send(message.bytes); }
        }));
        let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 32, ring: 1, speed: 1.0, rate: 60.0, ..Settings::default() };
        let playhead = Arc::new(Playhead::new(Arc::clone(&pages) as Arc<dyn Pages>, timeline, settings, Some(hook)).unwrap());
        let held = crate::host::held_tables(&*pages, pages.kinds.len());
        let host = PlayheadHost::new(Arc::clone(&playhead), |p| p, anchored, follow, held).unwrap();
        let shared: SharedView = TableView::Complete(replay_data::rows::TableRows::Countries(Vec::new())).into();
        let rows = Arc::new(FeedRows { playhead: Arc::clone(&playhead), shared, state: Arc::new(Mutex::new((None, None))) });
        let push = Arc::new(Push::default());
        host.show_panels(Arc::clone(&rows) as Arc<dyn PanelRows>, Arc::clone(&push));

        let mut seen = Vec::new();
        host.anchor(Anchor { seek: 1, frame_rate: 60.0, hour: 0, position: 0.0, timestamps: None, playing: false, speed: 1.0, sought: true });
        playhead.wait_newest(0).unwrap();

        host.prepare_panel(product(Arc::new(|hour, _, _, _| {
            let (live, value) = live_cell(0, 100, i64::from(hour) * 10);
            ProductRead { value, rows: 1, fresh: true, pending: false, live: Some(live), lanes: Vec::new(), from: Hour::MIN }
        }))).unwrap()(10, 1).unwrap();
        let _ = take_panel_data(&mut seen, &push, 10, 0);
        playhead.wait_declarations().unwrap();
        playhead.pace(16.0);
        playhead.wait_newest(8).unwrap();
        playhead.wait_declarations().unwrap();

        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let evals = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let epoch_before = playhead.epoch();
        host.prepare_panel(Box::new(GatedShownPanel {
            entered: entered_tx,
            release: Mutex::new(release_rx),
            evals: Arc::clone(&evals),
            live: None,
        })).unwrap()(43, 1).unwrap();
        rows.publish(0, 5, Some(0), false);
        entered_rx.recv().unwrap();

        host.follow().follow(99, Query::Cells { table: TABLE, columns: vec![0], rows: Rows::Keys(vec![100]) }).unwrap();
        loop {
            let bytes = follow_rx.recv().expect("external follow reset answer");
            if decode(&bytes).unwrap().answers.iter().any(|&(id, hour, _)| id == 99 && hour == 0) {
                break;
            }
        }
        assert_eq!(playhead.epoch(), epoch_before + 1, "external follower reset the paused playhead while Shown panel read was in flight");

        release_tx.send(()).unwrap();
        let data = take_panel_data(&mut seen, &push, 43, 0);
        assert!(
            !seen.iter().any(|(id, hour, kind, _)| *id == 43 && *kind == dated::STANDS && *hour == 5),
            "old-epoch STANDS(5) is stripped when the epoch advanced before the final guard"
        );
        assert_eq!(data["live"]["groups"][0]["seedHour"], 0);
        assert_eq!(data["sampledAt"], 0);
        rows.publish(0, 1, Some(0), true);
        loop {
            let (kind, bytes, session) = push.take_sent().expect("replacement frame packet");
            push.credit(session, u64::MAX);
            if kind != PANEL {
                continue;
            }
            let mut saw_stands = false;
            for r in dated::records(&bytes).unwrap() {
                if r.id == 43 && r.kind == dated::DATA {
                    panic!("equivalent same-hour replacement frame unexpectedly re-evaluated and sent DATA at hour {}", r.hour);
                }
                if r.id == 43 && r.kind == dated::STANDS && r.hour == 1 {
                    saw_stands = true;
                }
            }
            if saw_stands {
                break;
            }
        }
        assert_eq!(evals.load(std::sync::atomic::Ordering::Acquire), 1, "in-flight Shown panel published across the external paused reset and reused its result for an equivalent replacement frame");

        rows.publish(0, 1, Some(2), true);
        let updated = take_panel_data(&mut seen, &push, 43, 0);
        assert_eq!(updated["sampledAt"], 2, "same-hour replacement reusing the cached TableView pointer with a changed sampled_at must re-evaluate");
        assert_eq!(evals.load(std::sync::atomic::Ordering::Acquire), 2);
    }
}
