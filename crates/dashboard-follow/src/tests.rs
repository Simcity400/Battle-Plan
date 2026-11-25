use crate::{Answer, Follow, Identity, Message, Query, Rows, SeriesMode, Sink, State, StepStats, answer_at, decode};
use dashboard_declare::{Carry, Column, Key, Measure, Parts, Series};
use dashboard_query::{
    coverage::CoverageScope,
    identity::{Holder, HolderRun, Holders},
    timeline::{Join, Stretch, Timeline as Stretches},
};
use playhead::{
    Playhead, Settings, Timeline, Wanted,
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
    sync::{Arc, Mutex, mpsc},
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
}

fn harness(pages: &Arc<Recording>) -> Harness {
    stepping(pages, 24)
}

fn stepping(pages: &Arc<Recording>, every: Hour) -> Harness {
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
    let playhead = Arc::new(Playhead::new(Arc::clone(pages) as Arc<dyn Pages>, Timeline::whole(&**pages).unwrap(), settings, Some(hook)).unwrap());
    let reader = playhead.declare(Vec::new(), None).unwrap();
    let consumer = playhead.consumer(reader);
    let weak = Arc::downgrade(&playhead);
    follow.bind(move |tables, _joining| weak.upgrade().map_or(Ok(()), |playhead| playhead.redeclare(reader, tables)), playhead.log_reads()).unwrap();
    Harness { pages: Arc::clone(pages), playhead: Some(playhead), follow, consumer, played, head: 0, seen: Hour::MIN, every, reached: Hour::MIN, awaiting: false }
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

#[test]
fn an_answer_equal_to_the_last_is_not_sent() {
    let pages = record(quiet);
    let mut harness = harness(&pages);
    harness.follow.follow(7, keys(&[106], &[2])).unwrap();
    harness.seek(0);
    let played = harness.play(280);
    let first = played.iter().position(|played| !played.answers().is_empty()).expect("a first answer");
    let mut sent = played[first].head % 2;
    let mut unchanged = 0;
    for played in played[first + 1..].iter().filter(|played| played.moved() && played.head < 300) {
        assert_eq!(played.stats.evaluated, 1, "written at hours {}..{}", played.from, played.head);
        let differs = played.head % 2 != sent;
        assert_eq!(played.stats.changed, u32::from(differs), "hours {}..{}", played.from, played.head);
        assert_eq!(!played.answers().is_empty(), differs);
        unchanged += u32::from(!differs);
        sent = played.head % 2;
    }
    assert!(unchanged > 0, "an equal answer evaluated and not sent");
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
    assert_eq!((answer.keys[2], answer.values[2], answer.states[2]), (999, 0, State::Value));
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
    let reply = registry.answer(path, "follow", &follow_request(1, "country-stability", "stability"), |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    assert_eq!(reply["available"], false);
    assert_eq!(reply["reason"], "CountryStability is not in this playhead's files (a derived table, until format 12)");
    assert!(held.follow.declared().unwrap().is_empty(), "no reader made");
    let none = registry.answer(std::path::Path::new("b"), "follow", &follow_request(2, "country-measure", "politicalPower"), |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    assert_eq!((none["available"].clone(), none["reason"].clone()), (serde_json::json!(false), serde_json::json!("No playhead for this recording")));
    let alone = crate::registry::Registry::new(None).answer(path, "follow", &follow_request(3, "country-measure", "politicalPower"), |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    assert_eq!(alone["available"], false, "a process with no playheads follows nothing");
    let mut elsewhere = follow_request(5, "country-measure", "politicalPower");
    elsewhere["query"] = serde_json::json!({ "head": "campaign-clock", "params": { "branchId": "2", "firstHour": 200, "lastHour": 209 } });
    let refused = registry.answer(path, "follow", &elsewhere, |_| Ok(nobody()), Box::new(|_| panic!("a head naming another branch is never built"))).unwrap();
    assert_eq!((refused["available"].clone(), refused["reason"].clone()),
        (serde_json::json!(false), serde_json::json!("The followed query names branch 2, not the followed branch 1")));
    let followed = registry.answer(path, "follow", &follow_request(4, "country-measure", "politicalPower"), |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    assert_eq!(followed["available"], true);
    assert_eq!(held.follow.declared().unwrap(), vec![playhead::Declared { table: TABLE, columns: vec![2], keyed: false, passing: false, hourly: false }]);
    assert_eq!(registry.followed(), 1);
}

#[test]
fn two_tracks_of_one_timeline_at_different_anchors_each_follow_their_own_hours() {
    let (calendar, anchored) = (paper(), paper());
    let registry = crate::registry::Registry::new(Some(Arc::new(Papers(BTreeMap::from([("a".to_owned(), Arc::clone(&calendar)), ("a@1000".to_owned(), Arc::clone(&anchored))])))));
    let path = std::path::Path::new("a");
    let mut asked = follow_request(1, "country-measure", "politicalPower");
    registry.answer(path, "follow", &asked, |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    asked["id"] = 2.into();
    asked["clock"] = 1000.into();
    registry.answer(path, "follow", &asked, |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    assert_eq!((calendar.follow.declared().unwrap().len(), anchored.follow.declared().unwrap().len()), (1, 1), "each mapping's playhead reads its own follow");
    let anchor = |clock: i32, hour: i32| serde_json::json!({ "branchId": "1", "clock": clock, "seek": 12, "frameRate": 120, "hour": hour, "playing": false, "speed": 24.0, "sought": true });
    registry.answer(path, "follow-clock", &anchor(0, 10), |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    registry.answer(path, "follow-clock", &anchor(1000, 1010), |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    assert_eq!(calendar.anchors.lock().unwrap()[0].seek, 12);
    assert_eq!(calendar.anchors.lock().unwrap()[0].frame_rate, 120.0);
    let hours = |paper: &Paper| paper.anchors.lock().unwrap().iter().map(|anchor| anchor.hour).collect::<Vec<_>>();
    assert_eq!((hours(&calendar), hours(&anchored)), (vec![10], vec![1010]), "each at the hour its track shows");
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
    assert!(crate::parse(&unranked, &nobody()).is_err(), "a series asked for no tags ranks a top");
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
    let log =serde_json::json!({ "product": "log-page", "params": { "branchId": "1", "log": "market-events", "to": 9, "pageSize": 50, "columns": ["kind", "country"] } });
    let columns = vec![dashboard_declare::field(TableId::MarketEvents, "kind").unwrap(), dashboard_declare::field(TableId::MarketEvents, "country").unwrap()];
    match crate::parse(&log, &nobody()).unwrap() {
        Query::Cells { table, columns: asked, rows } => assert_eq!((table, asked, rows), (TableId::MarketEvents.index(), columns, Rows::Appended)),
        _ => panic!("a log's appended rows"),
    }
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
        assert_eq!(registry.answer(std::path::Path::new(path), "follow", &follow_request(id, "country-measure", "politicalPower"), |_| Ok(nobody()), Box::new(|_| None)).unwrap()["available"], true);
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
    use crate::push::{FAILURE, Push, failure};
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
    let notice = failure(&failed);
    assert_eq!(u32::from_le_bytes(notice[..4].try_into().unwrap()), FAILURE);
    assert_eq!(std::str::from_utf8(&notice[4..]).unwrap(), failed);
}

#[test]
fn push_queues_charge_one_budget_and_count_only_pressure_merges() {
    use crate::push::Push;
    let packet = one(7, 24, 1);
    let budget = playhead::ByteBudget::new(crate::answer::charge(&packet) * 2);
    let first = Push::with_budget(budget.clone());
    let second = Push::with_budget(budget.clone());
    first.offer(packet.clone());
    second.head(0, 0, 100);
    second.offer(packet.clone());
    second.offer(packet);
    assert_eq!(second.counters().pressure_merges, 0);
    second.offer(one(7, 48, 2));
    assert_eq!((second.counters().pressure_merges, second.counters().merges), (0, 2), "hours the clock passed merge before any pressure");
    assert!(second.failed().is_none());
    first.take_waiting();
    drop(second);
    assert_eq!(budget.used(), 0);
}

#[test]
fn a_merge_that_fails_sends_both_messages_as_they_are_in_order() {
    use crate::push::{Push, read_frame};
    let push = Push::default();
    push.offer(rows_message(10, false, &[(1, 10)]));
    push.offer(rows_message(10, true, &[(9, 1)]));
    for step in 0..3 {
        push.offer(one(7, 12 + step, i64::from(step)));
    }
    assert_eq!(push.counters().unmerged, 1);
    push.stop();
    let mut pipe = Vec::new();
    push.drain(&mut pipe).unwrap();
    let (mut input, mut payload, mut fours) = (pipe.as_slice(), Vec::new(), Vec::new());
    while read_frame(&mut input, &mut payload).unwrap().is_some() {
        fours.extend(decode(&payload).unwrap().answers.into_iter().filter(|&(id, ..)| id == 4).map(|(.., body)| Answer::parse(body).unwrap()));
    }
    assert_eq!(fours.iter().map(|answer| (answer.changed, answer.keys.clone())).collect::<Vec<_>>(), vec![(false, vec![1]), (true, vec![9])], "both reached the reader, in order");
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
    let (mut appended, mut forgotten) = (Vec::new(), Vec::new());
    replay_columnar::unit::reader::log_span(&pages.bytes, &run.pages, &LOG, TABLE, columns, run.first_hour, Hour::MIN..=Hour::MAX, &mut |hour, event| match event {
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
fn rows_changed_queued_behind_the_rows_they_change_are_laid_over_them_never_dropped() {
    let whole = rows_message(10, false, &[(1, 10), (2, 20), (3, 30)]);
    let first = rows_message(11, true, &[(2, 21)]);
    let second = rows_message(12, true, &[(3, 32), (2, 22)]);
    let (merged, superseded) = crate::answer::merge(&first, &second).unwrap();
    assert_eq!(superseded, 0, "rows changed are laid over, not superseded");
    let (_, _, body) = decode(&merged).unwrap().answers[0];
    let changed = Answer::parse(body).unwrap();
    assert!(changed.changed);
    assert_eq!((changed.keys.clone(), changed.values), (vec![2, 3], vec![22, 32]), "two changes are the newer of each row");
    let (merged, _) = crate::answer::merge(&whole, &merged).unwrap();
    let decoded = decode(&merged).unwrap();
    assert_eq!(decoded.hour, 12);
    let rows = Answer::parse(decoded.answers[0].2).unwrap();
    assert!(!rows.changed, "changes laid over whole rows are whole rows");
    assert_eq!((rows.keys, rows.values), (vec![1, 2, 3], vec![10, 22, 32]));
    assert!(crate::answer::merge(&rows_message(10, false, &[(1, 10)]), &rows_message(11, true, &[(9, 1)])).is_err(), "a change of a row not held is refused by name");
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
fn dated_panel_merge_keeps_the_reading_and_its_confirmation_separate() {
    use crate::panel::dated::{self, Update};
    let data = Ok(serde_json::json!({"rows": [7]}));
    let packet = |update| dated::message(4, 8, 2, &[update]);
    let (merged, _) = dated::merge(&packet(Update::Data(7, 10, &data)), &packet(Update::Stands(7, 24))).unwrap();
    let records = dated::records(&merged).unwrap();
    assert_eq!(records[0].body, b"\x81\xa4rows\x91\x07", "the worker reads MessagePack data records");
    assert_eq!(records.iter().map(|r| (r.kind, r.hour)).collect::<Vec<_>>(), [(dated::DATA, 10), (dated::STANDS, 24)]);
    assert!(dated::merge(&merged, &dated::message(4, 8, 3, &[Update::Stands(7, 30)])).is_err());
}

#[test]
fn dated_panel_merge_survives_a_waiting_product_and_coalesces_siblings() {
    use crate::panel::dated;
    let step = |hour, records: &[(u32, u8, &[u8])]| {
        let mut batch = dated::ProductBatch::new((0, 3, 9));
        for &(id, kind, body) in records {
            batch.changed(id, hour, kind, body);
        }
        batch.finish(Some(hour))
    };
    let (merged, superseded) = dated::merge(&step(10, &[(7, dated::DATA, b"v1")]), &step(34, &[(7, dated::WAITING, b"")])).unwrap();
    assert_eq!(superseded, 1);
    assert_eq!(
        dated::records(&merged).unwrap().iter().map(|r| (r.id, r.kind, r.hour, r.body)).collect::<Vec<_>>(),
        [(7, dated::DATA, 10, &b"v1"[..]), (7, dated::WAITING, 34, &b""[..]), (0, dated::COMPLETED, 34, &b""[..])]
    );
    let (merged, superseded) = dated::merge(
        &step(34, &[(7, dated::WAITING, b""), (8, dated::DATA, b"older-payload")]),
        &step(58, &[(7, dated::WAITING, b""), (8, dated::DATA, b"newer")]),
    ).unwrap();
    assert_eq!(superseded, 3);
    assert_eq!(
        dated::records(&merged).unwrap().iter().map(|r| (r.id, r.kind, r.hour, r.body)).collect::<Vec<_>>(),
        [(7, dated::WAITING, 34, &b""[..]), (8, dated::DATA, 58, &b"newer"[..]), (0, dated::COMPLETED, 58, &b""[..])]
    );
    let (merged, superseded) = dated::merge(&step(34, &[(7, dated::WAITING, b"")]), &step(58, &[(7, dated::DATA, b"ready")])).unwrap();
    assert_eq!(superseded, 1);
    assert_eq!(
        dated::records(&merged).unwrap().iter().map(|r| (r.id, r.kind, r.hour, r.body)).collect::<Vec<_>>(),
        [(7, dated::WAITING, 34, &b""[..]), (7, dated::DATA, 58, &b"ready"[..]), (0, dated::COMPLETED, 58, &b""[..])]
    );
    assert!(dated::merge(&step(10, &[(7, dated::DATA, b"x")]), &step(34, &[(8, dated::STANDS, b"")])).is_ok());
}

#[test]
fn passed_panel_messages_coalesce_with_waiting_and_interleaved_steps() {
    use crate::panel::dated;
    use crate::push::{FOLLOW, PANEL, Push};
    let panel_step = |hour, body: &[u8]| {
        let mut batch = dated::ProductBatch::new((0, 0, 0));
        batch.changed(7, hour, dated::WAITING, b"");
        batch.changed(8, hour, dated::DATA, body);
        batch.finish(Some(hour))
    };
    let payload = vec![b'a'; 512];
    let one_panel = panel_step(10, &payload);
    let one_step = rows_message(10, false, &[(1, 1)]);
    let budget = playhead::ByteBudget::new(1 << 20);
    let push = Push::with_budget(budget.clone());
    push.head(0, 0, 5);
    for (hour, value) in [(10, 1), (20, 2), (30, 3)] {
        push.offer_kind(FOLLOW, rows_message(hour, false, &[(1, value)]));
        push.offer_kind(PANEL, panel_step(hour, &payload));
    }
    assert_eq!(push.counters().merges, 0);
    assert_eq!(budget.used(), 3 * (crate::answer::charge(&one_step) + crate::answer::charge(&one_panel)));
    push.head(0, 0, 35);
    assert_eq!(push.counters().merges, 4);
    assert_eq!(push.counters().unmerged, 0);
    assert_eq!(budget.used(), crate::answer::charge(&one_step) + crate::answer::charge(&one_panel));
    let (first_kind, first_bytes, ..) = push.try_take().unwrap();
    let (second_kind, second_bytes, ..) = push.try_take().unwrap();
    assert!(push.try_take().is_none());
    let panel_bytes = if first_kind == PANEL { first_bytes } else { assert_eq!(second_kind, PANEL); second_bytes };
    let records = dated::records(&panel_bytes).unwrap();
    assert_eq!(
        records.iter().map(|r| (r.id, r.kind, r.hour)).collect::<Vec<_>>(),
        [(7, dated::WAITING, 10), (8, dated::DATA, 30), (0, dated::COMPLETED, 30)]
    );
}

#[test]
fn panel_budget_overflow_refuses_only_the_offending_panel() {
    use crate::panel::dated;
    use crate::push::{PANEL, Push};
    let mut batch = dated::ProductBatch::new((0, 0, 0));
    batch.changed(1, 40, dated::DATA, b"small-one");
    batch.changed(2, 40, dated::DATA, &vec![b'x'; 2048]);
    batch.changed(3, 40, dated::DATA, b"small-three");
    let packet = batch.finish(Some(40));
    let budget = playhead::ByteBudget::new(2048);
    let push = Push::with_budget(budget.clone());
    push.offer_kind(PANEL, packet);
    assert!(push.failed().is_none());
    assert!(budget.used() <= 2048);
    let (kind, sent, ..) = push.try_take().unwrap();
    assert_eq!(kind, PANEL);
    let records = dated::records(&sent).unwrap();
    assert_eq!(records.len(), 4);
    assert_eq!((records[0].id, records[0].kind, records[0].body), (1, dated::DATA, &b"small-one"[..]));
    assert_eq!((records[1].id, records[1].kind, records[1].stepped()), (2, dated::REFUSED, true));
    assert!(std::str::from_utf8(records[1].body).unwrap().contains("byte payload limit"));
    assert_eq!((records[2].id, records[2].kind, records[2].body), (3, dated::DATA, &b"small-three"[..]));
    assert_eq!((records[3].id, records[3].kind, records[3].hour), (0, dated::COMPLETED, 40));
    let mut next = dated::ProductBatch::new((0, 0, 0));
    next.changed(1, 50, dated::DATA, b"still-alive");
    push.offer_kind(PANEL, next.finish(Some(50)));
    assert!(push.failed().is_none());
    assert!(push.try_take().is_some());
}

#[test]
fn completed_product_step_has_constant_size_when_every_product_stands() {
    use crate::panel::dated;
    let mut batch = dated::ProductBatch::new((0, 3, 9));
    batch.changed(7, 10, dated::DATA, b"first");
    let first = batch.finish(Some(10));
    let next = dated::ProductBatch::new((0, 3, 9)).finish(Some(34));
    assert_eq!(dated::records(&next).unwrap().len(), 1);
    let (merged, _) = dated::merge(&first, &next).unwrap();
    let rows = dated::records(&merged).unwrap();
    assert_eq!(rows.iter().map(|r| (r.id, r.kind, r.hour)).collect::<Vec<_>>(), [(7, dated::DATA, 10), (0, dated::COMPLETED, 34)]);
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
fn rows_changed_twice_merge_in_key_order() {
    let merged = crate::answer::merge(&rows_message(10, true, &[(5, 1), (9, 1)]), &rows_message(11, true, &[(1, 2), (9, 2)])).unwrap().0;
    let decoded = decode(&merged).unwrap();
    let answer = Answer::parse(decoded.answers[0].2).unwrap();
    assert_eq!((answer.keys.clone(), answer.values.clone(), answer.canonical, answer.changed), (vec![1, 5, 9], vec![2, 1, 2], true, true));
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
fn a_changed_answer_reserves_the_table_it_lands_on_when_queued_so_sending_it_never_exceeds_the_budget() {
    use crate::push::Push;
    let small = routed(0, 5, 1, 0);
    let budget = playhead::ByteBudget::new(small.len() * 10);
    let push = Push::with_budget(budget.clone());
    push.offer(small.clone());
    assert!(push.try_take().is_some());
    push.head(0, 0, 100);
    push.offer(routed(0, 10, 2, (small.len() * 12) as u32));
    assert!(push.failed().is_some_and(|reason| reason.contains("byte payload limit")), "a table past the budget is refused by name when it is queued");
    assert!(push.try_take().is_some(), "the drain thread takes the failure notice and never panics");
    let push = Push::with_budget(budget);
    push.offer(routed(0, 10, 2, (small.len() * 4) as u32));
    assert_eq!(push.in_flight(), 0);
    assert!(push.try_take().is_some());
    assert_eq!(push.in_flight(), 24 + 16 + 2 * small.len() * 4, "what was reserved while queued is what is held in flight");
}

#[test]
fn a_reached_update_goes_past_a_future_one_of_another_route_but_never_past_its_own() {
    use crate::push::Push;
    let small = routed(1, 5, 1, 0);
    let push = Push::with_budget(playhead::ByteBudget::new(small.len() * 30));
    push.offer(small.clone());
    assert!(push.try_take().is_some());
    push.offer(routed(1, 50, 2, (small.len() * 8) as u32));
    assert!(push.try_take().is_none(), "the future update of route 1 waits for credit");
    push.offer(routed(2, 10, 3, 0));
    push.head(2, 0, 10);
    let (_, sent, ..) = push.try_take().expect("route 2's reached update goes past it");
    assert_eq!(crate::answer::decode(&sent).unwrap().route, 2);
    push.offer(routed(1, 11, 4, 0));
    push.head(1, 0, 11);
    assert!(push.try_take().is_none(), "route 1's later update never overtakes route 1's waiting one");
}

#[test]
fn an_answer_is_charged_at_the_size_its_lane_holds_and_merges_keep_that_size() {
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
    let merged = crate::answer::merge(&rows_message(10, true, &[(1, 1)]), &rows_message(11, true, &[(1, 2)])).unwrap().0;
    assert_eq!(crate::answer::decode(&merged).unwrap().held.len(), 1);
}

#[test]
fn only_hours_the_clock_has_passed_merge_into_one_answer() {
    use crate::push::Push;
    let push = Push::default();
    push.offer(rows_message(20, false, &[(1, 1)]));
    push.offer(rows_message(30, false, &[(1, 2)]));
    push.head(0, 0, 10);
    assert_eq!(push.counters().merges, 0, "never merged ahead of the clock");
    push.head(0, 0, 35);
    assert_eq!(push.counters().merges, 1, "both passed: one complete answer at the later hour");
    let (_, bytes, ..) = push.try_take().unwrap();
    let decoded = decode(&bytes).unwrap();
    assert_eq!((decoded.hour, Answer::parse(decoded.answers[0].2).unwrap().values), (30, vec![2]));
}

#[test]
fn pressure_never_merges_ahead_of_the_clock() {
    use crate::push::Push;
    let packet = rows_message(20, false, &[(1, 1)]);
    let budget = playhead::ByteBudget::new(crate::answer::charge(&packet) * 2);
    let push = Push::with_budget(budget);
    push.head(0, 0, 0);
    push.offer(rows_message(20, false, &[(1, 1)]));
    push.offer(rows_message(30, false, &[(1, 2)]));
    push.offer(rows_message(40, false, &[(1, 3)]));
    assert_eq!(push.counters().pressure_merges, 0);
    assert!(push.failed().is_some_and(|reason| reason.contains("byte payload limit")), "past the budget ahead of the clock is a named failure, never a lost hour");
}

#[test]
fn a_follow_names_lanes_its_id_belongs_to() {
    let registry = crate::registry::Registry::new(None);
    let ask = |params: serde_json::Value| registry.answer(std::path::Path::new("x"), "follow", &params, |_| Ok(nobody()), Box::new(|_| None));
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
    use crate::push::{FOLLOW, PANEL, Push};
    let registry = crate::registry::Registry::new(None);
    registry.restart("new page");
    let ask = |session: &str| registry.answer(std::path::Path::new("x"), "follow", &serde_json::json!({ "id": 7, "branchId": "1", "clock": 0, "session": session }),
        |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    assert_eq!(ask("old page")["reason"], "This page was replaced by a newer one; its follows ended with it", "a follow of the retired page changes nothing");
    assert_ne!(ask("new page")["reason"], ask("old page")["reason"], "the current page's follow goes on");

    let push = Push::default();
    push.head(0, 0, 1000);
    push.offer(at_seek(0, 1, 100, 1));
    push.offer(at_seek(0, 1, 124, 2));
    let (_, first, ..) = push.try_take().expect("the rewind's first answer");
    assert_eq!(crate::answer::decode(&first).unwrap().hour, 100, "a head of the old seek never merges away the new seek's destination answer");

    let push = Push::default();
    push.offer_kind(PANEL, crate::panel::dated::encoded(0, 0, 1, 3, 50, crate::panel::dated::STANDS, &[]));
    push.offer(at_seek(0, 2, 10, 3));
    let (kind, ..) = push.try_take().expect("the new seek's answer");
    assert!(kind == FOLLOW && push.try_take().is_none(), "an older seek's panel packet is retired with the follow answers of that seek");

    let small = at_seek(0, 0, 5, 1);
    let push = Push::with_budget(playhead::ByteBudget::new(crate::answer::charge(&small) * 2));
    let mut huge = Vec::new();
    crate::answer::begin(&mut huge, 0, 0, 6, 0, false);
    crate::answer::push_held(&mut huge, 7, 6, &body(7, 1), 1 << 20);
    push.offer(huge);
    assert!(push.failed().is_some() && push.try_take().is_some(), "past the budget the stream fails by name");
    push.restart();
    push.offer(small);
    assert!(push.failed().is_none() && push.try_take().is_some(), "a new page after an overflow plays again, with its drain still taking messages");
}

#[test]
fn review_8_one_session_token_binds_every_page_command_and_every_packet() {
    use crate::push::{FOLLOW, Push};
    let held = paper();
    let registry = crate::registry::Registry::new(Some(Arc::new(Papers(BTreeMap::from([("a".to_owned(), Arc::clone(&held))])))));
    let path = std::path::Path::new("a");
    registry.restart("page 1");
    let mut asked = follow_request(1, "country-measure", "politicalPower");
    asked["session"] = "page 1".into();
    assert_eq!(registry.answer(path, "follow", &asked, |_| Ok(nobody()), Box::new(|_| None)).unwrap()["available"], true);
    registry.restart("page 2");
    let clock = |session: &str| registry.answer(path, "follow-clock", &serde_json::json!({ "branchId": "1", "clock": 0, "frameRate": 60.0, "seek": 1, "hour": 5.0, "speed": 1.0, "session": session }),
        |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    asked["session"] = "page 2".into();
    asked["id"] = 2.into();
    registry.answer(path, "follow", &asked, |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    assert_eq!(clock("page 1")["reason"], "This page was replaced by a newer one; its follows ended with it", "a retired page's clock never anchors the new page's host");
    assert!(held.anchors.lock().unwrap().is_empty());
    assert_eq!(clock("page 2")["available"], true);

    let push = Push::default();
    let old = push.session();
    push.restart();
    push.offer_from(old, FOLLOW, at_seek(0, 0, 5, 1));
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
    holders.tags.insert("AAA".into(), vec![HolderRun { first: 0, last: HOURS as Hour - 1, holder: Holder::Found { slot: 103, id: 103 }, starts_segment: true }]);
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
    let names: Vec<Option<&str>> = (0..3).map(|row| answer.name(row)).collect();
    for (row, key) in answer.keys.iter().enumerate() {
        let wanted = if *key == 103 { "AAA".to_owned() } else { dashboard_query::identity::Country::Slot(u32::try_from(*key).unwrap()).key().into_owned() };
        assert_eq!(names[row], Some(wanted.as_str()), "row {row} is named by its tag or its slot");
    }
}
